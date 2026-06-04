use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use todo_curator::{check_closed_references, check_invalid, CheckOutput};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Parser)]
#[command(name = "todo-curator")]
#[command(about = "Check TODO comments against issue/MR status", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
enum Commands {
    #[command(about = "Check for TODO comments referencing closed issues or MRs")]
    CheckClosed {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long, value_enum, default_value = "text", help = "Output format")]
        format: OutputFormat,

        #[arg(short, long, help = "Output file (default: stdout)")]
        output: Option<PathBuf>,
    },

    #[command(about = "Check for improperly-formatted TODO comments")]
    CheckInvalid {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long, value_enum, default_value = "text", help = "Output format")]
        format: OutputFormat,

        #[arg(short, long, help = "Output file (default: stdout)")]
        output: Option<PathBuf>,
    },

    #[command(about = "Run all checks (check-closed + check-invalid)")]
    CheckAll {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long, value_enum, default_value = "text", help = "Output format")]
        format: OutputFormat,

        #[arg(short, long, help = "Output file (default: stdout)")]
        output: Option<PathBuf>,
    },

    #[command(about = "Check for TODO comments that should be removed when current MR closes")]
    CheckMrTodos {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long, value_enum, default_value = "text", help = "Output format")]
        format: OutputFormat,

        #[arg(short, long, help = "Output file (default: stdout)")]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::CheckClosed {
            path,
            format,
            output: output_path,
        } => {
            let result = check_closed_references(path).await?;
            output_and_exit(&result, format, output_path)?;
        }
        Commands::CheckInvalid {
            path,
            format,
            output: output_path,
        } => {
            let result = check_invalid(&path)?;
            output_and_exit(&result, format, output_path)?;
        }
        Commands::CheckAll {
            path,
            format,
            output: output_path,
        } => {
            let mut closed_result = check_closed_references(path.clone()).await?;
            let invalid_result = check_invalid(&path)?;
            closed_result.lint_violations = invalid_result.lint_violations;

            // Also run MR check (best-effort: skip if not in MR context)
            let mr_issues: Vec<MrIssue> = find_mr_todos(&path).await.unwrap_or_default();

            if closed_result.has_errors() || !mr_issues.is_empty() {
                closed_result.status = "failure".to_string();
            }

            if let Some(ref p) = output_path {
                let mut file = File::create(p)?;
                print_output(&mut file, &closed_result, format)?;
                print_mr_text_output(&mut file, &mr_issues, format)?;
            } else {
                let mut stdout = io::stdout();
                print_output(&mut stdout, &closed_result, format)?;
                print_mr_text_output(&mut stdout, &mr_issues, format)?;
            }

            if closed_result.has_errors() || !mr_issues.is_empty() {
                process::exit(1);
            }
        }
        Commands::CheckMrTodos {
            path,
            format,
            output: output_path,
        } => {
            check_mr_todos(path, format, output_path).await?;
        }
    }
    Ok(())
}

fn output_and_exit(
    result: &CheckOutput,
    format: OutputFormat,
    output_path: Option<PathBuf>,
) -> Result<()> {
    if let Some(path) = output_path {
        let mut file = File::create(path)?;
        print_output(&mut file, result, format)?;
    } else {
        let mut stdout = io::stdout();
        print_output(&mut stdout, result, format)?;
    }

    if result.has_errors() {
        process::exit(1);
    }
    Ok(())
}

fn print_output<W: Write>(
    writer: &mut W,
    output: &CheckOutput,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            // Use serde to serialize the output
            let json = serde_json::to_string_pretty(output)?;
            writeln!(writer, "{}", json)?;
        }
        OutputFormat::Text => {
            print_text_output(writer, output)?;
        }
    }
    Ok(())
}

/// JSON output format for MR TODO check results
#[derive(Debug, Serialize, Deserialize)]
struct MrTodosOutput {
    issues_closing: Vec<MrIssue>,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MrIssue {
    issue_number: u32,
    references: Vec<todo_curator::todo::TodoReference>,
}

fn print_text_output<W: Write>(writer: &mut W, output: &CheckOutput) -> Result<()> {
    if !output.closed.is_empty() {
        writeln!(
            writer,
            "{}",
            "TODO comments referencing closed issues/MRs:".red().bold()
        )?;
        for closed_ref in &output.closed {
            writeln!(
                writer,
                "{}: {}",
                closed_ref.reference.display().yellow(),
                closed_ref.title
            )?;
            writeln!(
                writer,
                "  {}:{}",
                closed_ref.reference.file_path().bold(),
                closed_ref.reference.line_number().to_string().bold()
            )?;
            let source = closed_ref.reference.source_line();
            if !source.is_empty() {
                writeln!(writer, "  {}", source.dimmed())?;
            }
        }
    }

    if !output.not_found.is_empty() {
        writeln!(
            writer,
            "\n{}",
            "TODO comments referencing non-existent or inaccessible issues/MRs:"
                .red()
                .bold()
        )?;
        for not_found_ref in &output.not_found {
            writeln!(
                writer,
                "{}: {}",
                not_found_ref.reference.display().yellow(),
                not_found_ref.error
            )?;
            writeln!(
                writer,
                "  {}:{}",
                not_found_ref.reference.file_path().bold(),
                not_found_ref.reference.line_number().to_string().bold()
            )?;
            let source = not_found_ref.reference.source_line();
            if !source.is_empty() {
                writeln!(writer, "  {}", source.dimmed())?;
            }
        }
    }

    if !output.has_errors() {
        writeln!(writer, "{}", "All TODO references are valid.".green())?;
    }

    if !output.lint_violations.is_empty() {
        let non_mergeable: Vec<_> = output
            .lint_violations
            .iter()
            .filter(|v| v.rule == "non-mergeable TODOs" || v.rule == "MVP comments")
            .collect();
        let bad_syntax: Vec<_> = output
            .lint_violations
            .iter()
            .filter(|v| v.rule != "non-mergeable TODOs" && v.rule != "MVP comments")
            .collect();

        if !bad_syntax.is_empty() {
            writeln!(
                writer,
                "\n{}",
                "Improperly-formatted TODO comments:".red().bold()
            )?;
            writeln!(
                writer,
                "  {}",
                r#"use "TODO [repo]#<ticket>", "TODO [repo]!<merge-request>", "TODO [group]&<epic>", or "TODO performance""#
                    .yellow()
            )?;
            for violation in &bad_syntax {
                writeln!(
                    writer,
                    "  {}:{}",
                    violation.file_path.bold(),
                    violation.line_number.to_string().bold(),
                )?;
                writeln!(writer, "    {}", violation.source_line.dimmed())?;
            }
        }

        if !non_mergeable.is_empty() {
            writeln!(writer, "\n{}", "Non-mergeable TODOs:".red().bold())?;
            for violation in &non_mergeable {
                writeln!(
                    writer,
                    "  {}:{}",
                    violation.file_path.bold(),
                    violation.line_number.to_string().bold(),
                )?;
                writeln!(writer, "    {}", violation.source_line.dimmed())?;
            }
        }
    }

    Ok(())
}

async fn find_mr_todos(path: &Path) -> Result<Vec<MrIssue>> {
    let checker = todo_curator::checker::StatusChecker::new().await?;
    checker.check_auth()?;

    let project = match todo_curator::checker::StatusChecker::detect_project(path) {
        todo_curator::checker::ProjectDetection::GitLab(proj) => proj,
        todo_curator::checker::ProjectDetection::GitHub(_) => {
            anyhow::bail!("GitHub PR TODO checking not yet implemented")
        }
        todo_curator::checker::ProjectDetection::None => {
            anyhow::bail!(
                "Could not detect project. Set CI_PROJECT_PATH or GITLAB_PROJECT environment variable."
            )
        }
    };

    let issues = checker.get_current_mr_issues(&project).await?;

    let extractor = todo_curator::todo::TodoExtractor::new();
    let all_references = extractor.extract_from_directory(path)?;

    let mut mr_issues = Vec::new();
    for issue_num in issues {
        let matching_refs: Vec<_> = all_references
            .iter()
            .filter(|r| match r {
                todo_curator::todo::TodoReference::GitLabIssue {
                    project: None,
                    number,
                    ..
                } => *number == issue_num,
                _ => false,
            })
            .cloned()
            .collect();

        if !matching_refs.is_empty() {
            mr_issues.push(MrIssue {
                issue_number: issue_num,
                references: matching_refs,
            });
        }
    }

    Ok(mr_issues)
}

fn print_mr_text_output<W: Write + ?Sized>(
    writer: &mut W,
    mr_issues: &[MrIssue],
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let mr_output = MrTodosOutput {
                issues_closing: mr_issues.to_vec(),
                status: if mr_issues.is_empty() {
                    "success".to_string()
                } else {
                    "failure".to_string()
                },
            };
            let json = serde_json::to_string_pretty(&mr_output)?;
            writeln!(writer, "{}", json)?;
        }
        OutputFormat::Text => {
            if mr_issues.is_empty() {
                writeln!(
                    writer,
                    "{}",
                    "No TODOs reference issues closed by this MR.".green()
                )?;
            } else {
                writeln!(
                    writer,
                    "{}",
                    "TODO comments that will be closed by this MR:"
                        .yellow()
                        .bold()
                )?;
                for mr_issue in mr_issues {
                    writeln!(
                        writer,
                        "{}: {} reference(s) found",
                        format!("#{}", mr_issue.issue_number).yellow(),
                        mr_issue.references.len()
                    )?;
                }
            }
        }
    }
    Ok(())
}

async fn check_mr_todos(
    path: PathBuf,
    format: OutputFormat,
    output_path: Option<PathBuf>,
) -> Result<()> {
    let mr_issues = find_mr_todos(&path).await?;

    let mut output: Box<dyn Write> = if let Some(path) = output_path {
        Box::new(File::create(path)?)
    } else {
        Box::new(io::stdout())
    };

    print_mr_text_output(&mut *output, &mr_issues, format)?;

    if !mr_issues.is_empty() {
        process::exit(1);
    }

    Ok(())
}
