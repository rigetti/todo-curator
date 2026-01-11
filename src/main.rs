use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use todo_curator::{check_closed_references, CheckOutput};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Parser)]
#[command(name = "todo-curator")]
#[command(about = "Check TODO comments against issue/MR status", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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

    #[command(about = "Check for TODO comments that should be removed when current MR closes")]
    CheckMrTodos {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long)]
        project: Option<String>,

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
        Commands::CheckClosed { path, format, output: output_path } => {
            let result = check_closed_references(path).await?;
            
            if let Some(path) = output_path {
                let mut file = File::create(path)?;
                print_output(&mut file, &result, format)?;
            } else {
                let mut stdout = io::stdout();
                print_output(&mut stdout, &result, format)?;
            }
            
            if result.has_errors() {
                process::exit(1);
            }
            Ok(())
        }
        Commands::CheckMrTodos { path, project, format, output: output_path } => {
            check_mr_todos(path, project, format, output_path).await
        }
    }
}

fn print_output<W: Write>(writer: &mut W, output: &CheckOutput, format: OutputFormat) -> Result<()> {
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
    Ok(())
}

async fn check_mr_todos(
    path: PathBuf,
    project: Option<String>,
    _format: OutputFormat,
    output_path: Option<PathBuf>,
) -> Result<()> {
    let mut output: Box<dyn Write> = if let Some(path) = output_path {
        Box::new(File::create(path)?)
    } else {
        Box::new(io::stdout())
    };
    let checker = todo_curator::checker::StatusChecker::new().await?;

    checker.check_auth()?;

    let project = project
        .or_else(|| std::env::var("GITLAB_PROJECT").ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "GitLab project path required. Set --project flag or GITLAB_PROJECT environment variable.\n\
                Example: --project group/subgroup/repo"
            )
        })?;

    let issues = checker.get_current_mr_issues(&project).await?;

    if issues.is_empty() {
        writeln!(output, "Not on an MR or no issues will be closed by current MR")?;
        return Ok(());
    }

    let extractor = todo_curator::todo::TodoExtractor::new();
    let all_references = extractor.extract_from_directory(&path)?;

    let mut found_todos = false;

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
            .collect();

        if !matching_refs.is_empty() {
            if !found_todos {
                writeln!(
                    output,
                    "{}",
                    "TODO comments that will be closed by this MR:"
                        .yellow()
                        .bold()
                )?;
                found_todos = true;
            }
            writeln!(
                output,
                "{}: {} reference(s) found",
                format!("#{}", issue_num).yellow(),
                matching_refs.len()
            )?;
        }
    }

    if found_todos {
        process::exit(1);
    }

    writeln!(output, "{}", "No TODOs reference issues closed by this MR.".green())?;
    Ok(())
}
