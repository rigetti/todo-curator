use crate::todo::TodoReference;
use anyhow::{Context, Result};
use gitlab::api::projects::{issues, merge_requests};
use gitlab::api::AsyncQuery;
use gitlab::AsyncGitlab;
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone)]
pub enum ProjectDetection {
    None,
    GitHub(String),
    GitLab(String),
}

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

#[derive(Debug, Serialize, Deserialize)]
pub struct ClosedReference {
    pub reference: TodoReference,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotFoundReference {
    pub reference: TodoReference,
    pub error: String,
}

pub struct CheckResult {
    pub closed: Vec<ClosedReference>,
    pub not_found: Vec<NotFoundReference>,
}

pub struct StatusChecker {
    github_client: Option<Octocrab>,
    gitlab_client: Option<AsyncGitlab>,
    github_token_present: bool,
    gitlab_token_present: bool,
}

impl StatusChecker {
    fn normalize_project_path(path: &str) -> String {
        path.trim_start_matches('/')
            .trim_end_matches(".git")
            .to_string()
    }

    /// Detect project from git remote origin URL
    /// Returns ProjectDetection enum indicating GitHub, GitLab, or no project
    pub fn detect_project(path: &std::path::Path) -> ProjectDetection {
        // Check for CI_PROJECT_PATH first (GitLab CI)
        if let Ok(ci_project_path) = env::var("CI_PROJECT_PATH") {
            return ProjectDetection::GitLab(Self::normalize_project_path(&ci_project_path));
        }

        // Try to discover git repository
        let repo = match gix::discover(path) {
            Ok(repo) => repo,
            Err(e) => {
                tracing::warn!(error=%e, "not in a valid git repository");
                return ProjectDetection::None;
            }
        };

        let remote = match repo.find_remote("origin") {
            Ok(remote) => remote,
            Err(e) => {
                tracing::warn!(error=%e, "failed to find origin remote");
                return ProjectDetection::None;
            }
        };

        let Some(url) = remote.url(gix::remote::Direction::Fetch) else {
            tracing::warn!("origin remote has no fetch URL");
            return ProjectDetection::None;
        };

        // Check host to determine GitHub vs GitLab
        match url.host() {
            Some("github.com") => {
                if let Some(path) = url.path_argument_safe() {
                    ProjectDetection::GitHub(Self::normalize_project_path(&path.to_string()))
                } else {
                    ProjectDetection::None
                }
            }
            Some("gitlab.com") => {
                if let Some(path) = url.path_argument_safe() {
                    ProjectDetection::GitLab(Self::normalize_project_path(&path.to_string()))
                } else {
                    ProjectDetection::None
                }
            }
            _ => ProjectDetection::None,
        }
    }

    pub async fn new() -> Result<Self> {
        let (github_client, gitlab_client, github_token_present, gitlab_token_present) =
            Self::init_clients().await?;

        Ok(Self {
            github_client,
            gitlab_client,
            github_token_present,
            gitlab_token_present,
        })
    }

    async fn init_clients() -> Result<(Option<Octocrab>, Option<AsyncGitlab>, bool, bool)> {
        let github_token = Self::github_token();
        let gitlab_token = Self::gitlab_token();

        let github_token_present = github_token.is_some();
        let gitlab_token_present = gitlab_token.is_some();

        let github_client = Self::init_github_client(github_token)?;
        let gitlab_client = Self::init_gitlab_client(gitlab_token).await?;

        Ok((
            github_client,
            gitlab_client,
            github_token_present,
            gitlab_token_present,
        ))
    }

    pub async fn validate_auth() -> Result<()> {
        let github_token = Self::github_token();
        if github_token.is_none() {
            anyhow::bail!("GitHub auth not configured: set GH_TOKEN or GITHUB_TOKEN");
        }

        let github_client =
            Self::init_github_client(github_token).context("Failed to initialize GitHub client")?;
        if github_client.is_none() {
            anyhow::bail!("Failed to initialize GitHub client");
        }

        let gitlab_token = Self::gitlab_token();
        if gitlab_token.is_none() {
            anyhow::bail!("GitLab auth not configured: set GITLAB_TOKEN or GL_TOKEN");
        }

        let gitlab_client = Self::init_gitlab_client(gitlab_token)
            .await
            .context("Failed to initialize GitLab client")?;
        if gitlab_client.is_none() {
            anyhow::bail!("Failed to initialize GitLab client");
        }

        Ok(())
    }

    fn github_token() -> Option<String> {
        env::var("GITHUB_TOKEN")
            .or_else(|_| env::var("GH_TOKEN"))
            .ok()
    }

    fn gitlab_token() -> Option<String> {
        env::var("GITLAB_TOKEN")
            .or_else(|_| env::var("GL_TOKEN"))
            .ok()
    }

    fn init_github_client(token: Option<String>) -> Result<Option<Octocrab>> {
        let Some(token) = token else {
            return Ok(None);
        };

        let octocrab = Octocrab::builder()
            .personal_token(token)
            .build()
            .context("Failed to initialize GitHub client")?;

        Ok(Some(octocrab))
    }

    fn normalized_gitlab_host() -> String {
        let gitlab_host = env::var("GITLAB_URL").unwrap_or_else(|_| "gitlab.com".to_string());

        let mut cleaned = gitlab_host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("ssh://")
            .trim_start_matches("git@");

        // Handle generic ssh://user@host/path form.
        if let Some((user, rest)) = cleaned.split_once('@') {
            if !user.contains('/') {
                cleaned = rest;
            }
        }

        // Handle scp-like SSH remotes: host:path/to/repo.git.
        if let Some(colon_idx) = cleaned.find(':') {
            let slash_idx = cleaned.find('/');

            if slash_idx.is_none_or(|idx| colon_idx < idx) {
                if let Some(idx) = slash_idx {
                    let after_colon = &cleaned[colon_idx + 1..idx];
                    if after_colon.chars().all(|c| c.is_ascii_digit()) {
                        // host:port/path
                        cleaned = &cleaned[..idx];
                    } else {
                        // host:path
                        cleaned = &cleaned[..colon_idx];
                    }
                }
            }
        }

        cleaned
            .split('/')
            .next()
            .unwrap_or(cleaned)
            .trim_end_matches('/')
            .to_string()
    }

    fn gitlab_token_type() -> String {
        env::var("GITLAB_TOKEN_TYPE")
            .or_else(|_| env::var("GL_TOKEN_TYPE"))
            .unwrap_or_else(|_| "pat".to_string())
            .to_ascii_lowercase()
    }

    async fn build_gitlab_client(
        host: &str,
        token: String,
        token_type: &str,
    ) -> Result<AsyncGitlab> {
        let mut builder = gitlab::GitlabBuilder::new(host, token);
        if matches!(token_type, "oauth" | "oauth2" | "bearer") {
            builder.oauth2_token();
        }

        builder
            .build_async()
            .await
            .context("Failed to initialize GitLab client")
    }

    async fn init_gitlab_client(token: Option<String>) -> Result<Option<AsyncGitlab>> {
        let Some(token) = token else {
            return Ok(None);
        };

        let gitlab_host = Self::normalized_gitlab_host();
        let token_type = Self::gitlab_token_type();
        let client = Self::build_gitlab_client(&gitlab_host, token, &token_type).await?;

        Ok(Some(client))
    }

    pub fn check_auth(&self) -> Result<()> {
        if self.github_client.is_none() && self.gitlab_client.is_none() {
            anyhow::bail!(
                "No authentication configured.\n\
                Set GH_TOKEN (or GITHUB_TOKEN) and/or GITLAB_TOKEN (or GL_TOKEN) environment variables with your personal access tokens.\n\
                - GitHub: https://github.com/settings/tokens (requires 'repo' scope)\n\
                - GitLab: https://gitlab.com/-/user_settings/personal_access_tokens (requires 'api' scope)\n\
                Optionally set GITLAB_URL for self-hosted GitLab (defaults to gitlab.com)."
            );
        }

        if self.github_client.is_none() && !self.github_token_present {
            tracing::warn!("GitHub token not set; GitHub TODO checking will be skipped.");
        }
        if self.gitlab_client.is_none() && !self.gitlab_token_present {
            tracing::warn!("GitLab token not set; GitLab TODO checking will be skipped.");
        }

        Ok(())
    }

    pub async fn check_references(
        &self,
        default_project: &ProjectDetection,
        references: &[TodoReference],
    ) -> Result<CheckResult> {
        let mut closed = Vec::new();
        let mut not_found = Vec::new();

        for reference in references {
            match self
                .check_single_reference(default_project, reference)
                .await
            {
                Ok(Some(closed_ref)) => closed.push(closed_ref),
                Ok(None) => {} // Reference exists but is not closed
                Err(e) => {
                    not_found.push(NotFoundReference {
                        reference: reference.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(CheckResult { closed, not_found })
    }

    async fn check_single_reference(
        &self,
        default_project: &ProjectDetection,
        reference: &TodoReference,
    ) -> Result<Option<ClosedReference>> {
        match reference {
            TodoReference::GitLabIssue {
                project, number, ..
            } => {
                self.check_gitlab_issue(default_project, reference, project.as_deref(), *number)
                    .await
            }
            TodoReference::GitHubIssue { repo, number, .. } => {
                self.check_github_issue(default_project, reference, repo.as_deref(), *number)
                    .await
            }
            TodoReference::GitLabMr {
                project, number, ..
            } => {
                self.check_gitlab_mr(default_project, reference, project.as_deref(), *number)
                    .await
            }
            TodoReference::GitHubPr { repo, number, .. } => {
                self.check_github_pr(reference, repo, *number).await
            }
            TodoReference::GitLabEpic { group, number, .. } => {
                self.check_gitlab_epic(default_project, reference, group.as_deref(), *number)
                    .await
            }
        }
    }

    async fn check_gitlab_issue(
        &self,
        default_project: &ProjectDetection,
        reference: &TodoReference,
        project: Option<&str>,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        let project_path = match project {
            Some(p) => p,
            None => match default_project {
                ProjectDetection::GitLab(p) => p.as_str(),
                _ => anyhow::bail!("GitLab issue requires project path"),
            },
        };

        let endpoint = issues::Issue::builder()
            .project(project_path)
            .issue(number as u64)
            .build()
            .context("Failed to build GitLab issue query")?;

        let issue: GitLabIssue = match endpoint.query_async(client).await {
            Ok(issue) => issue,
            Err(e) => {
                anyhow::bail!(
                    "GitLab issue not found or inaccessible (project {project_path}): {e}"
                );
            }
        };

        if issue.state == gitlab::webhooks::IssueState::Closed {
            Ok(Some(ClosedReference {
                reference: reference.clone(), // Preserves file_path and line_number from original
                title: issue.title,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_github_issue(
        &self,
        default_project: &ProjectDetection,
        reference: &TodoReference,
        repo: Option<&str>,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
        let Some(client) = self.github_client.as_ref() else {
            return Ok(None);
        };

        let repo_path = match repo {
            Some(r) => r,
            None => match default_project {
                ProjectDetection::GitHub(r) => r.as_str(),
                _ => anyhow::bail!("GitHub issue requires repository path"),
            },
        };

        let parts: Vec<&str> = repo_path.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid GitHub repo format: {}", repo_path);
        }
        let (owner, repo_name) = (parts[0], parts[1]);

        let issue = match client.issues(owner, repo_name).get(number as u64).await {
            Ok(issue) => issue,
            Err(e) => {
                anyhow::bail!("GitHub issue not found or inaccessible: {}", e);
            }
        };

        if issue.state == octocrab::models::IssueState::Closed {
            Ok(Some(ClosedReference {
                reference: reference.clone(),
                title: issue.title,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_gitlab_mr(
        &self,
        default_project: &ProjectDetection,
        reference: &TodoReference,
        project: Option<&str>,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        let project_path = match project {
            Some(p) => p,
            None => match default_project {
                ProjectDetection::GitLab(p) => p.as_str(),
                _ => anyhow::bail!("GitLab MR requires project path"),
            },
        };

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
                reference: reference.clone(),
                title: mr.title,
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_github_pr(
        &self,
        reference: &TodoReference,
        repo: &str,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
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
                reference: reference.clone(),
                title: pr.title.unwrap_or_else(|| "(no title)".to_string()),
            }))
        } else {
            Ok(None)
        }
    }

    async fn check_gitlab_epic(
        &self,
        default_project: &ProjectDetection,
        reference: &TodoReference,
        group: Option<&str>,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        // Determine the group path - for epics, we need to extract the group from the project path
        let group_path = match group {
            Some(g) => g.to_string(),
            None => {
                // For local epic references, we need to extract the group from the default project
                match default_project {
                    ProjectDetection::GitLab(project_path) => {
                        // Extract group from project path (e.g., "rigetti/qcs/services/myproject" -> "rigetti/qcs/services")
                        // Epics belong to groups, not projects, so we need to find the parent group
                        // For now, we'll try the full path minus the last component
                        let parts: Vec<&str> = project_path.split('/').collect();
                        if parts.len() > 1 {
                            parts[..parts.len() - 1].join("/")
                        } else {
                            anyhow::bail!(
                                "Cannot determine group from project path: {}",
                                project_path
                            )
                        }
                    }
                    _ => anyhow::bail!("GitLab epic requires group path"),
                }
            }
        };

        // Use a custom endpoint to query the GitLab epics API
        // The endpoint is: GET /groups/:id/epics/:epic_iid
        let endpoint = format!(
            "groups/{}/epics/{}",
            urlencoding::encode(&group_path),
            number
        );

        // Build HTTP request manually
        use gitlab::api::{AsyncClient, RestClient};
        let url = client
            .rest_endpoint(&endpoint)
            .context("Failed to build epic endpoint URL")?;

        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri(url.as_str());

        let response = client
            .rest_async(request, vec![])
            .await
            .context("Failed to query GitLab epic")?;

        // Check response status
        let status = response.status();

        // Parse the response body
        let body = response.into_body();
        let epic: serde_json::Value =
            serde_json::from_slice(&body).context("Failed to parse GitLab epic response")?;

        tracing::debug!("GitLab epic response (status {}): {:?}", status, epic);

        // Check if the response contains an error message (404, 403, etc.)
        if let Some(message) = epic.get("message").and_then(|v| v.as_str()) {
            anyhow::bail!("GitLab API error: {}", message);
        }

        // Parse the epic state
        let state = epic
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Epic response missing 'state' field"))?;

        let title = epic
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)")
            .to_string();

        tracing::debug!("Epic state: {}, title: {}", state, title);

        // Epics can be in states: opened, closed
        if state == "closed" {
            Ok(Some(ClosedReference {
                reference: reference.clone(),
                title,
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
        let repo = match gix::discover(".") {
            Ok(repo) => repo,
            Err(e) => {
                tracing::warn!(error=%e, "not in a valid git repository");
                return Ok(Vec::new());
            }
        };

        let head = match repo.head() {
            Ok(head) => head,
            Err(e) => {
                tracing::warn!(error=%e, "failed to get HEAD reference");
                return Ok(Vec::new());
            }
        };

        let branch = match head.referent_name() {
            Some(name) => name.shorten().to_string(),
            None => match Self::find_branch_pointing_at_head(&repo) {
                Ok(Some(branch)) => {
                    tracing::debug!(
                        "HEAD detached; selected branch pointing at HEAD: {}",
                        branch
                    );
                    branch
                }
                Ok(None) => {
                    tracing::warn!("HEAD is detached and no non-jj/keep branch points at HEAD");
                    return Ok(Vec::new());
                }
                Err(e) => {
                    tracing::warn!(error=%e, "failed to resolve branch name for detached HEAD");
                    return Ok(Vec::new());
                }
            },
        };

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
        let issue_regex =
            regex::Regex::new(r"(?i)(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)").unwrap();
        let issues: Vec<u32> = issue_regex
            .captures_iter(description)
            .filter_map(|caps| caps.get(1))
            .filter_map(|m| m.as_str().parse::<u32>().ok())
            .collect();

        Ok(issues)
    }

    /// Helper function for the common case when using `jj`:
    /// `git` is in headless mode, but the `HEAD` reference points to the branch we care about.
    fn find_branch_pointing_at_head(repo: &gix::Repository) -> Result<Option<String>> {
        let head_id = repo.head_id().context("failed to resolve HEAD id")?;
        let references = repo
            .references()
            .context("failed to initialize reference iteration")?;
        let mut branches = references
            .local_branches()
            .context("failed to iterate local branches")?;

        for reference in &mut branches {
            let reference =
                reference.map_err(|e| anyhow::anyhow!("failed to read branch reference: {e}"))?;
            let Some(reference_id) = reference.try_id() else {
                continue;
            };

            if reference_id != head_id {
                continue;
            }

            let branch_name = reference.name().shorten().to_string();
            if branch_name.starts_with("jj/keep") {
                continue;
            }

            return Ok(Some(branch_name));
        }

        Ok(None)
    }
}
