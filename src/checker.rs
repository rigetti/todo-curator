use crate::todo::TodoReference;
use anyhow::{Context, Result};
use gitlab::api::projects::{issues, merge_requests};
use gitlab::api::AsyncQuery;
use gitlab::AsyncGitlab;
use octocrab::Octocrab;
use serde::Deserialize;
use std::env;

#[derive(Debug, Deserialize)]
struct GitLabIssue {
    title: String,
    state: gitlab::webhooks::IssueState,
}

#[derive(Debug, Deserialize)]
struct GitLabMergeRequest {
    title: String,
    state: gitlab::webhooks::MergeRequestState,
    description: Option<String>,
}

#[derive(Debug)]
pub struct ClosedReference {
    pub reference: TodoReference,
    pub title: String,
}

pub struct StatusChecker {
    github_client: Option<Octocrab>,
    gitlab_client: Option<AsyncGitlab>,
}

impl StatusChecker {
    pub async fn new() -> Result<Self> {
        let github_client = Self::init_github_client()?;
        let gitlab_client = Self::init_gitlab_client().await?;

        Ok(Self {
            github_client,
            gitlab_client,
        })
    }

    fn init_github_client() -> Result<Option<Octocrab>> {
        if let Ok(token) = env::var("GITHUB_TOKEN") {
            let octocrab = Octocrab::builder()
                .personal_token(token)
                .build()
                .context("Failed to build GitHub client")?;
            Ok(Some(octocrab))
        } else {
            Ok(None)
        }
    }

    async fn init_gitlab_client() -> Result<Option<AsyncGitlab>> {
        if let Ok(token) = env::var("GITLAB_TOKEN") {
            let gitlab_host = env::var("GITLAB_URL").unwrap_or_else(|_| "https://gitlab.com".to_string());
            match gitlab::GitlabBuilder::new(&gitlab_host, token)
                .build_async()
                .await
            {
                Ok(client) => Ok(Some(client)),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize GitLab client: {}", e);
                    eprintln!("GitLab TODO checking will be skipped.");
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    pub fn check_auth(&self) -> Result<()> {
        if self.gitlab_client.is_none() && self.github_client.is_none() {
            anyhow::bail!(
                "No authentication configured.\n\
                Set GITHUB_TOKEN and/or GITLAB_TOKEN environment variables with your personal access tokens.\n\
                - GitHub: https://github.com/settings/tokens (requires 'repo' scope)\n\
                - GitLab: https://gitlab.com/-/user_settings/personal_access_tokens (requires 'api' scope)\n\
                Optionally set GITLAB_URL for self-hosted GitLab (defaults to gitlab.com)."
            );
        }

        Ok(())
    }

    pub async fn check_references(&self, references: &[TodoReference]) -> Result<Vec<ClosedReference>> {
        let mut closed = Vec::new();

        for reference in references {
            if let Some(closed_ref) = self.check_single_reference(reference).await? {
                closed.push(closed_ref);
            }
        }

        Ok(closed)
    }

    async fn check_single_reference(&self, reference: &TodoReference) -> Result<Option<ClosedReference>> {
        match reference {
            TodoReference::GitLabIssue { project, number } => {
                self.check_gitlab_issue(project.as_deref(), *number).await
            }
            TodoReference::GitHubIssue { repo, number } => {
                self.check_github_issue(repo, *number).await
            }
            TodoReference::GitLabMr { project, number } => {
                self.check_gitlab_mr(project.as_deref(), *number).await
            }
            TodoReference::GitHubPr { repo, number } => {
                self.check_github_pr(repo, *number).await
            }
        }
    }

    async fn check_gitlab_issue(&self, project: Option<&str>, number: u32) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        let project_path = project.context("GitLab issue requires project path")?;
        
        let endpoint = issues::Issue::builder()
            .project(project_path)
            .issue(number as u64)
            .build()
            .context("Failed to build GitLab issue query")?;

        let issue: GitLabIssue = match endpoint.query_async(client).await {
            Ok(issue) => issue,
            Err(_) => return Ok(None),
        };

        if issue.state == gitlab::webhooks::IssueState::Closed {
            Ok(Some(ClosedReference {
                reference: TodoReference::GitLabIssue {
                    project: Some(project_path.to_string()),
                    number,
                },
                title: issue.title,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_github_issue(&self, repo: &str, number: u32) -> Result<Option<ClosedReference>> {
        let Some(client) = self.github_client.as_ref() else {
            return Ok(None);
        };

        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid GitHub repo format: {}", repo);
        }
        let (owner, repo_name) = (parts[0], parts[1]);

        let issue = match client.issues(owner, repo_name).get(number as u64).await {
            Ok(issue) => issue,
            Err(_) => return Ok(None),
        };

        if issue.state == octocrab::models::IssueState::Closed {
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

    async fn check_gitlab_mr(&self, project: Option<&str>, number: u32) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        let project_path = project.context("GitLab MR requires project path")?;
        
        let endpoint = merge_requests::MergeRequest::builder()
            .project(project_path)
            .merge_request(number as u64)
            .build()
            .context("Failed to build GitLab MR query")?;

        let mr: GitLabMergeRequest = match endpoint.query_async(client).await {
            Ok(mr) => mr,
            Err(_) => return Ok(None),
        };

        if mr.state != gitlab::webhooks::MergeRequestState::Opened {
            Ok(Some(ClosedReference {
                reference: TodoReference::GitLabMr {
                    project: Some(project_path.to_string()),
                    number,
                },
                title: mr.title,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_github_pr(&self, repo: &str, number: u32) -> Result<Option<ClosedReference>> {
        let Some(client) = self.github_client.as_ref() else {
            return Ok(None);
        };

        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid GitHub repo format: {}", repo);
        }
        let (owner, repo_name) = (parts[0], parts[1]);

        let pr = match client.pulls(owner, repo_name).get(number as u64).await {
            Ok(pr) => pr,
            Err(_) => return Ok(None),
        };

        if pr.state != Some(octocrab::models::IssueState::Open) {
            Ok(Some(ClosedReference {
                reference: TodoReference::GitHubPr {
                    repo: repo.to_string(),
                    number,
                },
                title: pr.title.unwrap_or_else(|| "(no title)".to_string()),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_current_mr_issues(&self, project: &str) -> Result<Vec<u32>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(Vec::new());
        };

        // Get current branch name to find the MR
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .context("Failed to get current git branch")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        
        // Find MR for this branch
        let endpoint = merge_requests::MergeRequests::builder()
            .project(project)
            .source_branch(&branch)
            .state(merge_requests::MergeRequestState::Opened)
            .build()
            .context("Failed to build GitLab MR query")?;

        let mrs: Vec<GitLabMergeRequest> = match endpoint.query_async(client).await {
            Ok(mrs) => mrs,
            Err(_) => return Ok(Vec::new()),
        };

        if mrs.is_empty() {
            return Ok(Vec::new());
        }

        // Get the first MR and extract issue numbers from description
        let mr = &mrs[0];
        let description = mr.description.as_deref().unwrap_or("");
        
        // Parse "Closes #123" or "Fixes #456" patterns
        let issue_regex = regex::Regex::new(r"(?i)(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)").unwrap();
        let issues: Vec<u32> = issue_regex
            .captures_iter(description)
            .filter_map(|caps| caps.get(1))
            .filter_map(|m| m.as_str().parse::<u32>().ok())
            .collect();

        Ok(issues)
    }
}
