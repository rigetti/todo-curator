use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use todo_curator::{
    check_closed_from_extraction, check_closed_references, check_invalid,
    check_invalid_from_extraction, checker::ProjectDetection, checker::StatusChecker,
    extract_todos, CheckOutput,
};

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

#[derive(clap::Args)]
struct Args {
    #[arg(short, long, default_value = ".", env = "TODO_CURATOR_PATH")]
    path: PathBuf,

    #[arg(
        long,
        value_enum,
        default_value = "text",
        env = "TODO_CURATOR_FORMAT",
        help = "Output format"
    )]
    format: OutputFormat,

    #[arg(
        short,
        long,
        env = "TODO_CURATOR_OUTPUT",
        help = "Output file (default: stdout)"
    )]
    output: Option<PathBuf>,

    #[arg(
        long,
        action = clap::ArgAction::Append,
        value_delimiter = ',',
        env = "TODO_CURATOR_EXCLUDE_FILE_REGEX",
        help = "Regex patterns for files to exclude from linting (repeat flag or provide comma-separated values)"
    )]
    exclude_file_regex: Vec<String>,
}

#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
enum Commands {
    #[command(about = "Check for TODO comments referencing closed issues or MRs")]
    CheckClosed {
        #[command(flatten)]
        args: Args,
    },

    #[command(about = "Check for improperly-formatted TODO comments")]
    CheckInvalid {
        #[command(flatten)]
        args: Args,
    },

    #[command(about = "Run all checks (check-closed + check-invalid)")]
    CheckAll {
        #[command(flatten)]
        args: Args,
    },

    #[command(about = "Check for TODO comments that should be removed when current MR closes")]
    CheckMrTodos {
        #[command(flatten)]
        args: Args,
    },

    #[command(about = "Validate that both GitHub and GitLab clients initialize")]
    ValidateAuth,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    if matches!(cli.command, Commands::ValidateAuth) {
        todo_curator::checker::StatusChecker::validate_auth().await?;
        println!("Authentication validated for GitHub and GitLab.");
        return Ok(());
    }

    let path = match &cli.command {
        Commands::CheckClosed { args }
        | Commands::CheckInvalid { args }
        | Commands::CheckAll { args }
        | Commands::CheckMrTodos { args } => args.path.clone(),
        Commands::ValidateAuth => unreachable!(),
    };

    let project_detection = StatusChecker::detect_project(&path);
    match &project_detection {
        ProjectDetection::GitLab(project) => {
            tracing::debug!("Detected GitLab project: {project}");
        }
        ProjectDetection::GitHub(repo) => {
            tracing::debug!("Detected GitHub repo: {repo}");
        }
        ProjectDetection::None => {
            tracing::error!("No project detected!");
        }
    };

    let checker = StatusChecker::new().await?;

    let needs_auth = matches!(
        cli.command,
        Commands::CheckClosed { .. } | Commands::CheckAll { .. } | Commands::CheckMrTodos { .. }
    );

    if needs_auth {
        checker.check_auth()?;
    }

    match cli.command {
        Commands::CheckClosed { args } => {
            let Args {
                path,
                format,
                output: output_path,
                exclude_file_regex,
            } = args;
            let result =
                check_closed_references(path, &project_detection, &checker, &exclude_file_regex)
                    .await?;
            output_and_exit(&result, format, output_path)?;
        }
        Commands::CheckInvalid { args } => {
            let Args {
                path,
                format,
                output: output_path,
                exclude_file_regex,
            } = args;
            let result = check_invalid(&path, &project_detection, &checker, &exclude_file_regex)?;
            output_and_exit(&result, format, output_path)?;
        }
        Commands::CheckAll { args } => {
            let Args {
                path,
                format,
                output: output_path,
                exclude_file_regex,
            } = args;
            let extraction = extract_todos(&path, &exclude_file_regex)?;
            let mut closed_result = check_closed_from_extraction(
                &extraction,
                &project_detection,
                &checker,
            )
            .await?;
            let invalid_result =
                check_invalid_from_extraction(&extraction, &project_detection)?;
            for (category, mut violations) in invalid_result.lint_violations {
                closed_result
                    .lint_violations
                    .entry(category)
                    .or_default()
                    .append(&mut violations);
            }

            // Also run MR check (best-effort: skip if not in MR context)
            let mr_issues: Vec<MrIssue> = find_mr_todos(
                &extraction.references,
                &project_detection,
                &checker,
            )
            .await
            .unwrap_or_default();

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
        Commands::CheckMrTodos { args } => {
            let Args {
                path,
                format,
                output: output_path,
                exclude_file_regex,
            } = args;
            let extraction = extract_todos(&path, &exclude_file_regex)?;
            check_mr_todos(
                &extraction.references,
                format,
                output_path,
                &project_detection,
                &checker,
            )
            .await?;
        }
        Commands::ValidateAuth => unreachable!(),
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

    if !output.warnings.is_empty() {
        writeln!(
            writer,
            "\n{}",
            "TODO references that can be shortened:".yellow().bold()
        )?;
        for warning in &output.warnings {
            writeln!(
                writer,
                "{} -> {}",
                warning.original.yellow(),
                warning.suggestion.green()
            )?;
            writeln!(
                writer,
                "  {}:{}",
                warning.reference.file_path().bold(),
                warning.reference.line_number().to_string().bold()
            )?;
        }
    }

    if !output.has_errors() {
        writeln!(writer, "{}", "All TODO references are valid.".green())?;
    }

    if !output.lint_violations.is_empty() {
        for (category, violations) in &output.lint_violations {
            writeln!(writer, "\n{}", category.header().red().bold())?;
            if let Some(hint) = category.header_hint() {
                writeln!(writer, "  {}", hint.yellow())?;
            }
            for violation in violations {
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

async fn find_mr_todos(
    references: &std::collections::HashSet<todo_curator::todo::TodoReference>,
    project_detection: &ProjectDetection,
    checker: &StatusChecker,
) -> Result<Vec<MrIssue>> {
    let project = match project_detection {
        ProjectDetection::GitLab(proj) => proj.clone(),
        ProjectDetection::GitHub(_) => {
            anyhow::bail!("GitHub PR TODO checking not yet implemented")
        }
        ProjectDetection::None => {
            anyhow::bail!(
                "Could not detect project. Set CI_PROJECT_PATH or GITLAB_PROJECT environment variable."
            )
        }
    };

    let issues = checker.get_current_mr_issues(&project).await?;

    let mut mr_issues = Vec::new();
    for issue_num in issues {
        let matching_refs: Vec<_> = references
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
    references: &std::collections::HashSet<todo_curator::todo::TodoReference>,
    format: OutputFormat,
    output_path: Option<PathBuf>,
    project_detection: &ProjectDetection,
    checker: &StatusChecker,
) -> Result<()> {
    let mr_issues = find_mr_todos(references, project_detection, checker).await?;

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
