use crate::todo::TodoReference;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug)]
pub struct ClosedReference {
    pub reference: TodoReference,
    pub title: String,
}

pub struct StatusChecker;

impl StatusChecker {
    pub fn new() -> Self {
        Self
    }

    pub fn check_auth(&self) -> Result<()> {
        let glab_status = Command::new("glab")
            .arg("auth")
            .arg("status")
            .output()
            .context("Failed to run 'glab auth status'")?;

        if !glab_status.status.success() {
            anyhow::bail!(
                "'glab' is either not working or not authorized.\n\
                See https://docs.gitlab.com/editor_extensions/gitlab_cli/#authenticate-with-gitlab"
            );
        }

        let gh_status = Command::new("gh")
            .arg("auth")
            .arg("status")
            .output()
            .context("Failed to run 'gh auth status'")?;

        if !gh_status.status.success() {
            anyhow::bail!(
                "'gh' is either not working or not authorized.\n\
                See https://cli.github.com/manual/gh_auth_login"
            );
        }

        Ok(())
    }

    pub fn check_references(&self, references: &[TodoReference]) -> Result<Vec<ClosedReference>> {
        let mut closed = Vec::new();

        for reference in references {
            if let Some(closed_ref) = self.check_single_reference(reference)? {
                closed.push(closed_ref);
            }
        }

        Ok(closed)
    }

    fn check_single_reference(&self, reference: &TodoReference) -> Result<Option<ClosedReference>> {
        match reference {
            TodoReference::GitLabIssue { project, number } => {
                self.check_gitlab_issue(project.as_deref(), *number)
            }
            TodoReference::GitHubIssue { repo, number } => {
                self.check_github_issue(repo, *number)
            }
            TodoReference::GitLabMr { project, number } => {
                self.check_gitlab_mr(project.as_deref(), *number)
            }
            TodoReference::GitHubPr { repo, number } => {
                self.check_github_pr(repo, *number)
            }
        }
    }

    fn check_gitlab_issue(&self, project: Option<&str>, number: u32) -> Result<Option<ClosedReference>> {
        let mut cmd = Command::new("glab");
        cmd.arg("issue");
        
        if project.is_some() {
            cmd.arg("view");
        } else {
            cmd.arg("show");
        }
        
        if let Some(proj) = project {
            cmd.arg(format!("{}#{}", proj, number));
            cmd.arg("--repo").arg(proj);
        } else {
            cmd.arg(number.to_string());
        }

        let output = cmd.output().context("Failed to run glab issue command")?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        if stdout.contains("state: closed") || stdout.contains("state:\tclosed") {
            let title = extract_title_from_glab(&stdout);
            Ok(Some(ClosedReference {
                reference: TodoReference::GitLabIssue {
                    project: project.map(String::from),
                    number,
                },
                title,
            }))
        } else {
            Ok(None)
        }
    }

    fn check_github_issue(&self, repo: &str, number: u32) -> Result<Option<ClosedReference>> {
        let issue_ref = format!("{}#{}", repo, number);
        
        let output = Command::new("gh")
            .arg("issue")
            .arg("view")
            .arg(&issue_ref)
            .arg("--json")
            .arg("state,title")
            .output()
            .context("Failed to run gh issue view")?;

        if !output.status.success() {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct GhIssue {
            state: String,
            title: String,
        }

        let issue: GhIssue = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gh issue output")?;

        if issue.state == "CLOSED" {
            Ok(Some(ClosedReference {
                reference: TodoReference::GitHubIssue {
                    repo: repo.to_string(),
                    number,
                },
                title: issue.title,
            }))
        } else {
            Ok(None)
        }
    }

    fn check_gitlab_mr(&self, project: Option<&str>, number: u32) -> Result<Option<ClosedReference>> {
        let mut cmd = Command::new("glab");
        cmd.arg("mr");
        
        if project.is_some() {
            cmd.arg("view");
        } else {
            cmd.arg("show");
        }
        
        if let Some(proj) = project {
            cmd.arg(number.to_string());
            cmd.arg("--repo").arg(proj);
        } else {
            cmd.arg(number.to_string());
        }

        let output = cmd.output().context("Failed to run glab mr command")?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        let is_open = stdout.contains("state: open") || stdout.contains("state:\topen");
        
        if !is_open {
            let title = extract_title_from_glab(&stdout);
            Ok(Some(ClosedReference {
                reference: TodoReference::GitLabMr {
                    project: project.map(String::from),
                    number,
                },
                title,
            }))
        } else {
            Ok(None)
        }
    }

    fn check_github_pr(&self, repo: &str, number: u32) -> Result<Option<ClosedReference>> {
        let pr_ref = format!("{}#{}", repo, number);
        
        let output = Command::new("gh")
            .arg("pr")
            .arg("view")
            .arg(&pr_ref)
            .arg("--json")
            .arg("state,title")
            .output()
            .context("Failed to run gh pr view")?;

        if !output.status.success() {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct GhPr {
            state: String,
            title: String,
        }

        let pr: GhPr = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gh pr output")?;

        if pr.state != "OPEN" {
            Ok(Some(ClosedReference {
                reference: TodoReference::GitHubPr {
                    repo: repo.to_string(),
                    number,
                },
                title: pr.title,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_current_mr_issues(&self) -> Result<Vec<u32>> {
        let output = Command::new("glab")
            .arg("mr")
            .arg("issues")
            .output()
            .context("Failed to run 'glab mr issues'")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let issue_regex = regex::Regex::new(r"^#(\d+)").unwrap();
        
        let issues: Vec<u32> = stdout
            .lines()
            .filter_map(|line| {
                issue_regex.captures(line)
                    .and_then(|caps| caps.get(1))
                    .and_then(|m| m.as_str().parse::<u32>().ok())
            })
            .collect();

        Ok(issues)
    }
}

fn extract_title_from_glab(output: &str) -> String {
    let title_regex = regex::Regex::new(r"title:\s+(.+)").unwrap();
    
    title_regex
        .captures(output)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| String::from("(no title)"))
}
