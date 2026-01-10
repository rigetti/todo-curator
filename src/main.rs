mod checker;
mod todo;

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use std::process;

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
    },
    
    #[command(about = "Check for TODO comments that should be removed when current MR closes")]
    CheckMrTodos {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CheckClosed { path } => check_closed_references(path),
        Commands::CheckMrTodos { path } => check_mr_todos(path),
    }
}

fn check_closed_references(path: PathBuf) -> Result<()> {
    let checker = checker::StatusChecker::new();
    
    checker.check_auth()?;

    let extractor = todo::TodoExtractor::new();
    let references = extractor.extract_from_directory(&path)?;

    if references.is_empty() {
        println!("No TODO references found.");
        return Ok(());
    }

    let references_vec: Vec<_> = references.into_iter().collect();
    let closed = checker.check_references(&references_vec)?;

    if !closed.is_empty() {
        eprintln!("{}", "TODO comments referencing closed issues/MRs:".red().bold());
        for closed_ref in &closed {
            eprintln!("{}: {}", closed_ref.reference.display().red(), closed_ref.title);
        }
        process::exit(1);
    }

    println!("{}", "All TODO references are valid.".green());
    Ok(())
}

fn check_mr_todos(path: PathBuf) -> Result<()> {
    let checker = checker::StatusChecker::new();
    
    checker.check_auth()?;

    let issues = checker.get_current_mr_issues()?;
    
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
                todo::TodoReference::GitLabIssue { project: None, number } => *number == issue_num,
                _ => false,
            })
            .collect();

        if !matching_refs.is_empty() {
            if !found_todos {
                eprintln!("{}", "TODO comments that will be closed by this MR:".yellow().bold());
                found_todos = true;
            }
            eprintln!("{}: {} reference(s) found", format!("#{}", issue_num).yellow(), matching_refs.len());
        }
    }

    if found_todos {
        process::exit(1);
    }

    println!("{}", "No TODOs reference issues closed by this MR.".green());
    Ok(())
}
