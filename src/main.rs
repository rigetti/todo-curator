mod checker;
mod todo;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use std::path::PathBuf;
use std::process;

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
    },

    #[command(about = "Check for TODO comments that should be removed when current MR closes")]
    CheckMrTodos {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(long)]
        project: Option<String>,

        #[arg(long, value_enum, default_value = "text", help = "Output format")]
        format: OutputFormat,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::CheckClosed { path, format } => check_closed_references(path, format).await,
        Commands::CheckMrTodos { path, project, format } => check_mr_todos(path, project, format).await,
    }
}

async fn check_closed_references(path: PathBuf, format: OutputFormat) -> Result<()> {
    // Detect GitLab project from git origin for local TODO references
    let gitlab_project = checker::StatusChecker::detect_gitlab_project(&path);
    tracing::debug!("GitLab project: {gitlab_project:?}");
    let checker = checker::StatusChecker::with_default_project(gitlab_project).await?;

    checker.check_auth()?;

    let extractor = todo::TodoExtractor::new();
    let references = extractor.extract_from_directory(&path)?;

    if references.is_empty() {
        match format {
            OutputFormat::Json => {
                println!("{}", serde_json::json!({"closed": [], "not_found": [], "status": "success"}));
            }
            OutputFormat::Text => {
                println!("No TODO references found.");
            }
        }
        return Ok(());
    }

    let references_vec: Vec<_> = references.into_iter().collect();
    let result = checker.check_references(&references_vec).await?;

    let mut has_errors = false;

    match format {
        OutputFormat::Json => {
            // JSON output
            let status = if result.closed.is_empty() && result.not_found.is_empty() {
                "success"
            } else {
                "failure"
            };
            let output = serde_json::json!({
                "closed": result.closed,
                "not_found": result.not_found,
                "status": status
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
            
            if !result.closed.is_empty() || !result.not_found.is_empty() {
                process::exit(1);
            }
        }
        OutputFormat::Text => {
        // Human-readable output
        if !result.closed.is_empty() {
            eprintln!(
                "{}",
                "TODO comments referencing closed issues/MRs:".red().bold()
            );
            for closed_ref in &result.closed {
                eprintln!(
                    "{}: {}",
                    closed_ref.reference.display().yellow(),
                    closed_ref.title
                );
                eprintln!(
                    "  {}:{}",
                    closed_ref.reference.file_path().bold(),
                    closed_ref.reference.line_number().to_string().bold()
                );
                let source = closed_ref.reference.source_line();
                if !source.is_empty() {
                    eprintln!("  {}", source.dimmed());
                }
            }
            has_errors = true;
        }

        if !result.not_found.is_empty() {
            eprintln!(
                "\n{}",
                "TODO comments referencing non-existent or inaccessible issues/MRs:"
                    .red()
                    .bold()
            );
            for not_found_ref in &result.not_found {
                eprintln!(
                    "{}: {}",
                    not_found_ref.reference.display().yellow(),
                    not_found_ref.error
                );
                eprintln!(
                    "  {}:{}",
                    not_found_ref.reference.file_path().bold(),
                    not_found_ref.reference.line_number().to_string().bold()
                );
                let source = not_found_ref.reference.source_line();
                if !source.is_empty() {
                    eprintln!("  {}", source.dimmed());
                }
            }
            has_errors = true;
        }

        if has_errors {
            process::exit(1);
        }

            println!("{}", "All TODO references are valid.".green());
        }
    }
    Ok(())
}

async fn check_mr_todos(path: PathBuf, project: Option<String>, _format: OutputFormat) -> Result<()> {
    let checker = checker::StatusChecker::new().await?;

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
        println!("Not on an MR or no issues will be closed by current MR");
        return Ok(());
    }

    let extractor = todo::TodoExtractor::new();
    let all_references = extractor.extract_from_directory(&path)?;

    let mut found_todos = false;

    for issue_num in issues {
        let matching_refs: Vec<_> = all_references
            .iter()
            .filter(|r| match r {
                todo::TodoReference::GitLabIssue {
                    project: None,
                    number,
                    ..
                } => *number == issue_num,
                _ => false,
            })
            .collect();

        if !matching_refs.is_empty() {
            if !found_todos {
                eprintln!(
                    "{}",
                    "TODO comments that will be closed by this MR:"
                        .yellow()
                        .bold()
                );
                found_todos = true;
            }
            eprintln!(
                "{}: {} reference(s) found",
                format!("#{}", issue_num).yellow(),
                matching_refs.len()
            );
        }
    }

    if found_todos {
        process::exit(1);
    }

    println!("{}", "No TODOs reference issues closed by this MR.".green());
    Ok(())
}
