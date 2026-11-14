use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use todo_curator::todo::TodoReference;
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
#[command(arg_required_else_help = true)]
struct Cli {
    /// Print the usage spec (<https://usage.jdx.dev>) for this CLI and exit
    #[arg(long, hide = true)]
    usage_spec: bool,

    #[command(subcommand)]
    command: Option<Commands>,
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
        env = "TODO_CURATOR_EXCLUDE_FILE_REGEX",
        help = "Regex pattern for files to exclude from linting"
    )]
    exclude_file_regex: Option<String>,
}

#[derive(Subcommand)]
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

/// The subcommands that scan a directory for TODOs.
#[derive(Debug, Clone, Copy)]
enum CheckKind {
    Closed,
    Invalid,
    All,
    MrTodos,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    if cli.usage_spec {
        clap_usage::generate(&mut Cli::command(), "todo-curator", &mut io::stdout());
        return Ok(());
    }
    let command = cli
        .command
        .expect("clap requires a subcommand unless --usage-spec is given");

    let (kind, args) = match command {
        Commands::ValidateAuth => {
            StatusChecker::validate_auth().await?;
            println!("Authentication validated for GitHub and GitLab.");
            return Ok(());
        }
        Commands::CheckClosed { args } => (CheckKind::Closed, args),
        Commands::CheckInvalid { args } => (CheckKind::Invalid, args),
        Commands::CheckAll { args } => (CheckKind::All, args),
        Commands::CheckMrTodos { args } => (CheckKind::MrTodos, args),
    };

    let project_detection = StatusChecker::detect_project(&args.path);
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
    }

    match kind {
        CheckKind::Closed => run_check_closed(args, &project_detection).await,
        CheckKind::Invalid => run_check_invalid(args, &project_detection),
        CheckKind::All => run_check_all(args, &project_detection).await,
        CheckKind::MrTodos => run_check_mr_todos(args, &project_detection).await,
    }
}

async fn authenticated_checker() -> Result<StatusChecker> {
    let checker = StatusChecker::new().await?;
    checker.check_auth()?;
    Ok(checker)
}

async fn run_check_closed(args: Args, project_detection: &ProjectDetection) -> Result<()> {
    let checker = authenticated_checker().await?;
    let exclude_file_regex = args.exclude_file_regex.as_deref().unwrap_or("");
    let result = check_closed_references(
        args.path.clone(),
        project_detection,
        &checker,
        exclude_file_regex,
    )
    .await?;
    output_and_exit(&result, args.format, args.output)
}

fn run_check_invalid(args: Args, project_detection: &ProjectDetection) -> Result<()> {
    let exclude_file_regex = args.exclude_file_regex.as_deref().unwrap_or("");
    let result = check_invalid(&args.path, project_detection, exclude_file_regex)?;
    output_and_exit(&result, args.format, args.output)
}

async fn run_check_all(args: Args, project_detection: &ProjectDetection) -> Result<()> {
    let checker = authenticated_checker().await?;
    let exclude_file_regex = args.exclude_file_regex.as_deref().unwrap_or("");
    let extraction = extract_todos(&args.path, exclude_file_regex)?;
    let mut closed_result =
        check_closed_from_extraction(&extraction, project_detection, &checker).await?;
    let invalid_result = check_invalid_from_extraction(&extraction, project_detection)?;
    closed_result
        .lint_violations
        .extend(invalid_result.lint_violations);

    // Also run MR check (best-effort: skip if not in MR context)
    let mr_issues: Vec<MrIssue> =
        find_mr_todos(&extraction.references, project_detection, &checker)
            .await
            .unwrap_or_default();

    if closed_result.has_errors() || !mr_issues.is_empty() {
        closed_result.status = "failure".to_string();
    }

    if let Some(ref p) = args.output {
        let mut file = File::create(p)?;
        print_output(&mut file, &closed_result, args.format)?;
        print_mr_text_output(&mut file, &mr_issues, args.format)?;
    } else {
        let mut stdout = io::stdout();
        print_output(&mut stdout, &closed_result, args.format)?;
        print_mr_text_output(&mut stdout, &mr_issues, args.format)?;
    }

    if closed_result.has_errors() || !mr_issues.is_empty() {
        process::exit(1);
    }
    Ok(())
}

async fn run_check_mr_todos(args: Args, project_detection: &ProjectDetection) -> Result<()> {
    let checker = authenticated_checker().await?;
    let exclude_file_regex = args.exclude_file_regex.as_deref().unwrap_or("");
    let extraction = extract_todos(&args.path, exclude_file_regex)?;
    check_mr_todos(
        &extraction.references,
        args.format,
        args.output,
        project_detection,
        &checker,
    )
    .await
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
            writeln!(writer, "{json}")?;
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
    references: Vec<TodoReference>,
}

fn write_location<W: Write>(writer: &mut W, reference: &TodoReference) -> Result<()> {
    writeln!(
        writer,
        "  {}:{}",
        reference.file_path().bold(),
        reference.line_number().to_string().bold()
    )?;
    Ok(())
}

fn write_source_line<W: Write>(writer: &mut W, reference: &TodoReference) -> Result<()> {
    let source = reference.source_line();
    if !source.is_empty() {
        writeln!(writer, "  {}", source.dimmed())?;
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

        let mut grouped: BTreeMap<String, (&str, Vec<&todo_curator::checker::ClosedReference>)> =
            BTreeMap::new();
        for closed_ref in &output.closed {
            let key = closed_ref.reference.display();
            grouped
                .entry(key)
                .and_modify(|(_, refs)| refs.push(closed_ref))
                .or_insert_with(|| (closed_ref.title.as_str(), vec![closed_ref]));
        }

        for (reference_display, (title, refs)) in grouped {
            writeln!(writer, "{}: {}", reference_display.yellow(), title)?;
            for closed_ref in refs {
                write_location(writer, &closed_ref.reference)?;
                write_source_line(writer, &closed_ref.reference)?;
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
            write_location(writer, &not_found_ref.reference)?;
            write_source_line(writer, &not_found_ref.reference)?;
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
            write_location(writer, &warning.reference)?;
        }
    }

    if !output.has_errors() {
        writeln!(writer, "{}", "All TODO references are valid.".green())?;
    }

    if !output.lint_violations.is_empty() {
        writeln!(writer, "\n{}", "Invalid TODO-like comments:".red().bold())?;
        writeln!(
            writer,
            "  {}",
            r"use `TODO (<ref>)`, where `<ref>` is `[repo]#<ticket>`, `<group>#<work-item>`, `[repo]!<merge-request>`, `[group]&<epic>`, or `performance`"
                .yellow()
        )?;
        for violation in &output.lint_violations {
            writeln!(
                writer,
                "  {}:{}",
                violation.file_path.bold(),
                violation.line_number.to_string().bold(),
            )?;
            writeln!(writer, "    {}", violation.source_line.dimmed())?;
        }
    }

    Ok(())
}

async fn find_mr_todos(
    references: &std::collections::HashSet<TodoReference>,
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
            .filter(|r| match &r.kind {
                todo_curator::todo::TodoReferenceKind::GitLabIssue {
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
            writeln!(writer, "{json}")?;
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
    references: &std::collections::HashSet<TodoReference>,
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
