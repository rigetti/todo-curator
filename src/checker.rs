use crate::todo::{TodoReference, TodoReferenceKind};
use anyhow::{Context, Result};
use gitlab::api::projects::{issues, merge_requests};
use gitlab::api::{AsyncClient, AsyncQuery, RestClient};
use gitlab::AsyncGitlab;
use octocrab::Octocrab;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone)]
pub enum ProjectDetection {
    None,
    GitHub(String),
    GitLab(String),
}

impl ProjectDetection {
    /// Return true when this detection is GitLab and the project path matches exactly.
    #[must_use]
    pub fn matches_gitlab(&self, project_path: &str) -> bool {
        match self {
            Self::GitLab(project) => project == project_path,
            _ => false,
        }
    }

    /// Return true when this detection is GitLab and its parent group matches the expected group.
    #[must_use]
    pub fn parent_group_matches(&self, group_path: &str) -> bool {
        match self {
            Self::GitLab(project) => project
                .rsplit_once('/')
                .is_some_and(|(parent, _)| parent == group_path),
            _ => false,
        }
    }
}

/// Issue state as returned by the GitLab REST API.
///
/// `gitlab` only offers query-filter enums for this, which do not implement `Deserialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum IssueState {
    Opened,
    Closed,
}

/// Merge request state as returned by the GitLab REST API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MergeRequestState {
    Opened,
    Closed,
    Locked,
    Merged,
}

#[derive(Debug, Deserialize)]
struct GitLabIssue {
    title: String,
    state: IssueState,
}

#[derive(Debug, Deserialize)]
struct GitLabMergeRequest {
    title: String,
    state: MergeRequestState,
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

/// How a GitLab token is presented to the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitlabAuthKind {
    /// `PRIVATE-TOKEN` header.
    Pat,
    /// `Authorization: Bearer` header.
    OAuth2,
    /// `JOB-TOKEN` header (CI job token).
    Job,
}

impl std::str::FromStr for GitlabAuthKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pat" => Ok(Self::Pat),
            "oauth" | "oauth2" | "bearer" => Ok(Self::OAuth2),
            "job" => Ok(Self::Job),
            other => anyhow::bail!(
                "Unknown Gitlab token type {other:?}; expected one of: pat, oauth2 (oauth, bearer), job"
            ),
        }
    }
}

/// GitLab credentials read from the environment.
#[derive(Debug, Clone)]
struct GitlabAuth {
    kind: GitlabAuthKind,
    token: SecretString,
}

impl GitlabAuth {
    /// Read the GitLab token and auth scheme from the environment,
    /// or `None` if no token is set.
    ///
    /// The token is `GITLAB_TOKEN`, else `GL_TOKEN`, else `CI_JOB_TOKEN`.
    /// The scheme is `GITLAB_TOKEN_TYPE` (or `GL_TOKEN_TYPE`).
    /// A missing type means `pat`,
    /// except when the only token is `CI_JOB_TOKEN`, which means `job`.
    /// An unrecognized type is an error,
    /// as is an explicit non-`job` type with `CI_JOB_TOKEN` as the only token:
    /// an explicit type means a token of that type was expected but is missing.
    fn from_env() -> Result<Option<Self>> {
        let env_var = StatusChecker::env_var;

        let (token, only_ci_job_token) =
            match env_var("GITLAB_TOKEN").or_else(|| env_var("GL_TOKEN")) {
                Some(token) => (token, false),
                None => match env_var("CI_JOB_TOKEN") {
                    Some(token) => (token, true),
                    None => return Ok(None),
                },
            };

        let kind = match env_var("GITLAB_TOKEN_TYPE").or_else(|| env_var("GL_TOKEN_TYPE")) {
            Some(value) => {
                let kind: GitlabAuthKind = value.parse()?;
                if only_ci_job_token && kind != GitlabAuthKind::Job {
                    anyhow::bail!(
                        "Explicit GitLab token type is {value:?}, but GITLAB_TOKEN (or GL_TOKEN) is not set or is empty \
                         and only CI_JOB_TOKEN is available. \
                         Set GITLAB_TOKEN, or set GITLAB_TOKEN_TYPE=job to use the CI job token."
                    );
                }
                kind
            }
            None if only_ci_job_token => GitlabAuthKind::Job,
            None => GitlabAuthKind::Pat,
        };

        Ok(Some(Self {
            kind,
            token: token.into(),
        }))
    }

    /// Build a GitLab client for `host` that authenticates with this token and scheme.
    async fn build(self, host: &str) -> Result<AsyncGitlab> {
        // `gitlab` takes the token only as a plain `String`.
        let mut builder = gitlab::GitlabBuilder::new(host, self.token.expose_secret());
        match self.kind {
            GitlabAuthKind::Pat => {}
            GitlabAuthKind::OAuth2 => {
                builder.oauth2_token();
            }
            GitlabAuthKind::Job => {
                builder.job_token();
            }
        }

        builder
            .build_async()
            .await
            .context("Failed to initialize GitLab client")
    }
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
    /// Returns `ProjectDetection` enum indicating GitHub, GitLab, or no project
    pub fn detect_project(path: &std::path::Path) -> ProjectDetection {
        // Check for CI_PROJECT_PATH first (GitLab CI)
        if let Some(ci_project_path) = Self::env_var("CI_PROJECT_PATH") {
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
            Some("github.com") => url
                .path_argument_safe()
                .map_or(ProjectDetection::None, |path| {
                    ProjectDetection::GitHub(Self::normalize_project_path(&path.to_string()))
                }),
            Some("gitlab.com") => url
                .path_argument_safe()
                .map_or(ProjectDetection::None, |path| {
                    ProjectDetection::GitLab(Self::normalize_project_path(&path.to_string()))
                }),
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
        let gitlab_auth = GitlabAuth::from_env()?;

        let github_token_present = github_token.is_some();
        let gitlab_token_present = gitlab_auth.is_some();

        let github_client = Self::init_github_client(github_token)?;
        let gitlab_client = Self::init_gitlab_client(gitlab_auth).await?;

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

        let gitlab_auth = GitlabAuth::from_env().context("Failed to initialize GitLab client")?;
        if gitlab_auth.is_none() {
            anyhow::bail!("GitLab auth not configured: set GITLAB_TOKEN or GL_TOKEN");
        }

        let gitlab_client = Self::init_gitlab_client(gitlab_auth)
            .await
            .context("Failed to initialize GitLab client")?;
        if gitlab_client.is_none() {
            anyhow::bail!("Failed to initialize GitLab client");
        }

        Ok(())
    }

    /// Read an environment variable, treating a value that is empty or entirely
    /// whitespace as if the variable were unset.
    ///
    /// CI systems routinely expand an unset secret to an empty string, and an
    /// empty token is never usable: taking it at face value turns "no
    /// credentials for this forge" into a 401 from that forge.
    fn env_var(name: &str) -> Option<String> {
        let value = env::var(name).ok()?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            tracing::debug!(name, "ignoring environment variable set to an empty value");
            return None;
        }
        Some(trimmed.to_string())
    }

    fn github_token() -> Option<SecretString> {
        Self::env_var("GITHUB_TOKEN")
            .or_else(|| Self::env_var("GH_TOKEN"))
            .map(SecretString::from)
    }

    fn init_github_client(token: Option<SecretString>) -> Result<Option<Octocrab>> {
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
        let gitlab_host = Self::env_var("GITLAB_URL").unwrap_or_else(|| "gitlab.com".to_string());

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

    async fn init_gitlab_client(auth: Option<GitlabAuth>) -> Result<Option<AsyncGitlab>> {
        let Some(auth) = auth else {
            return Ok(None);
        };

        Ok(Some(auth.build(&Self::normalized_gitlab_host()).await?))
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
                Ok(Some(closed_ref)) => {
                    tracing::info!("Reference closed: {:?}", closed_ref.reference);
                    closed.push(closed_ref);
                }
                Ok(None) => {
                    tracing::info!("Reference exists but is not closed: {:?}", reference);
                }
                Err(e) => {
                    tracing::warn!(
                        "Reference not found or inaccessible: {:?}, error: {}",
                        reference,
                        e
                    );
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
        match &reference.kind {
            TodoReferenceKind::GitLabIssue {
                project, number, ..
            } => {
                self.check_gitlab_issue(default_project, reference, project.as_deref(), *number)
                    .await
            }
            TodoReferenceKind::GitHubIssueOrPr { repo, number, .. } => {
                self.check_github_issue(default_project, reference, repo.as_deref(), *number)
                    .await
            }
            TodoReferenceKind::GitLabMr {
                project, number, ..
            } => {
                self.check_gitlab_mr(default_project, reference, project.as_deref(), *number)
                    .await
            }
            TodoReferenceKind::GitHubPr { repo, number, .. } => {
                self.check_github_pr(reference, repo, *number).await
            }
            TodoReferenceKind::GitLabEpic { group, number, .. } => {
                self.check_gitlab_epic(default_project, reference, group.as_deref(), *number)
                    .await
            }
            TodoReferenceKind::GitLabWorkItem { group, number } => {
                self.check_gitlab_work_item(reference, group, *number).await
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
            .issue(u64::from(number))
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

        if issue.state == IssueState::Closed {
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
            anyhow::bail!("Invalid GitHub repo format: {repo_path}");
        }
        let (owner, repo_name) = (parts[0], parts[1]);

        let issue = match client.issues(owner, repo_name).get(u64::from(number)).await {
            Ok(issue) => issue,
            Err(e) => {
                anyhow::bail!("GitHub issue not found or inaccessible: {e}");
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
            .merge_request(u64::from(number))
            .build()
            .context("Failed to build GitLab MR query")?;

        let mr: GitLabMergeRequest = match endpoint.query_async(client).await {
            Ok(mr) => mr,
            Err(_) => return Ok(None),
        };

        if mr.state != MergeRequestState::Opened {
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
            anyhow::bail!("Invalid GitHub repo format: {repo}");
        }
        let (owner, repo_name) = (parts[0], parts[1]);

        let Ok(pr) = client.pulls(owner, repo_name).get(u64::from(number)).await else {
            return Ok(None);
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
                                "Cannot determine group from project path: {project_path}"
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

        // Parse the response body
        let body = response.into_body();
        let epic: serde_json::Value =
            serde_json::from_slice(&body).context("Failed to parse GitLab epic response")?;

        // Check if the response contains an error message (404, 403, etc.)
        if let Some(message) = epic.get("message").and_then(|v| v.as_str()) {
            anyhow::bail!("GitLab API error: {message}");
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

    /// Check a group-level work item via GraphQL.
    ///
    /// GitLab has no REST endpoint for group work items,
    /// so this queries `namespace(fullPath:) { workItem(iid:) { title state } }`.
    async fn check_gitlab_work_item(
        &self,
        reference: &TodoReference,
        group: &str,
        number: u32,
    ) -> Result<Option<ClosedReference>> {
        let Some(client) = self.gitlab_client.as_ref() else {
            return Ok(None);
        };

        let url = client
            .instance_endpoint("api/graphql")
            .context("Failed to build GraphQL endpoint URL")?;
        let body = serde_json::to_vec(&Self::work_item_query(group, number))?;
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri(url.as_str())
            .header(http::header::CONTENT_TYPE, "application/json");

        let response = client
            .rest_async(request, body)
            .await
            .context("Failed to query GitLab work item")?;
        let status = response.status();
        let payload: serde_json::Value =
            serde_json::from_slice(response.body()).with_context(|| {
                format!("Failed to parse GitLab work item response (HTTP {status})")
            })?;

        let (state, title) = Self::parse_work_item_response(&payload, group, number)?;
        if state == "CLOSED" {
            Ok(Some(ClosedReference {
                reference: reference.clone(),
                title,
            }))
        } else {
            Ok(None)
        }
    }

    fn work_item_query(group: &str, number: u32) -> serde_json::Value {
        serde_json::json!({
            "query": "query($fullPath: ID!, $iid: String!) { namespace(fullPath: $fullPath) { workItem(iid: $iid) { title state } } }",
            "variables": { "fullPath": group, "iid": number.to_string() },
        })
    }

    /// Extract `(state, title)` from a GraphQL work item response.
    ///
    /// A missing namespace or work item, or any GraphQL error, is an error (not found).
    fn parse_work_item_response(
        payload: &serde_json::Value,
        group: &str,
        number: u32,
    ) -> Result<(String, String)> {
        if let Some(errors) = payload.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let messages: Vec<&str> = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect();
                anyhow::bail!("GitLab GraphQL error: {}", messages.join("; "));
            }
        }
        if let Some(message) = payload.get("message").and_then(|m| m.as_str()) {
            anyhow::bail!("GitLab API error: {message}");
        }

        let work_item = payload
            .pointer("/data/namespace/workItem")
            .filter(|v| !v.is_null())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "GitLab work item not found or inaccessible (group {group}, #{number})"
                )
            })?;
        let state = work_item
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Work item response missing 'state' field"))?;
        let title = work_item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)");
        Ok((state.to_string(), title.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Run `f` with exactly the given environment variables set.
    #[expect(clippy::result_large_err)] // `figment::Error` is dictated by `Jail`'s API.
    fn with_env<R>(vars: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
        let mut out = None;
        figment::Jail::expect_with(|jail| {
            jail.clear_env();
            for (k, v) in vars {
                jail.set_env(k, v);
            }
            out = Some(f());
            Ok(())
        });
        out.unwrap()
    }

    fn auth(vars: &[(&str, &str)]) -> Option<GitlabAuth> {
        with_env(vars, GitlabAuth::from_env).unwrap()
    }

    fn kind(vars: &[(&str, &str)]) -> GitlabAuthKind {
        auth(vars).expect("a GitLab token").kind
    }

    fn token(vars: &[(&str, &str)]) -> Option<String> {
        auth(vars).map(|auth| auth.token.expose_secret().to_owned())
    }

    #[test]
    fn auth_debug_output_hides_token() {
        let auth = auth(&[("GITLAB_TOKEN", "glpat-very-secret")]).expect("a GitLab token");
        let debug = format!("{auth:?}");
        assert!(!debug.contains("glpat-very-secret"), "{debug}");
    }

    #[test]
    fn kind_parses_known_names_case_insensitively() {
        for (name, kind) in [
            ("pat", GitlabAuthKind::Pat),
            ("PAT", GitlabAuthKind::Pat),
            ("oauth", GitlabAuthKind::OAuth2),
            ("oauth2", GitlabAuthKind::OAuth2),
            ("Bearer", GitlabAuthKind::OAuth2),
            ("job", GitlabAuthKind::Job),
        ] {
            assert_eq!(name.parse::<GitlabAuthKind>().unwrap(), kind, "{name}");
        }
    }

    #[test]
    fn kind_rejects_unknown_names() {
        assert!("jbo".parse::<GitlabAuthKind>().is_err());
    }

    #[test]
    fn env_token_type_job_selects_job() {
        let vars = [("GITLAB_TOKEN", "t"), ("GITLAB_TOKEN_TYPE", "job")];
        assert_eq!(kind(&vars), GitlabAuthKind::Job);
    }

    #[test]
    fn env_token_type_is_case_insensitive_and_has_gl_alias() {
        let vars = [("GL_TOKEN", "t"), ("GL_TOKEN_TYPE", "JOB")];
        assert_eq!(kind(&vars), GitlabAuthKind::Job);
    }

    #[test]
    fn env_missing_or_blank_type_means_pat() {
        assert_eq!(kind(&[("GITLAB_TOKEN", "t")]), GitlabAuthKind::Pat);
        let vars = [("GITLAB_TOKEN", "t"), ("GITLAB_TOKEN_TYPE", " ")];
        assert_eq!(kind(&vars), GitlabAuthKind::Pat);
    }

    #[test]
    fn env_oauth_aliases_select_oauth2() {
        for ty in ["oauth", "oauth2", "bearer"] {
            let vars = [("GITLAB_TOKEN", "t"), ("GITLAB_TOKEN_TYPE", ty)];
            assert_eq!(kind(&vars), GitlabAuthKind::OAuth2, "{ty}");
        }
    }

    #[test]
    fn env_unknown_token_type_is_an_error() {
        let vars = [("GITLAB_TOKEN", "t"), ("GITLAB_TOKEN_TYPE", "jbo")];
        assert!(with_env(&vars, GitlabAuth::from_env).is_err());
    }

    #[test]
    fn env_explicit_non_job_type_with_only_ci_job_token_is_an_error() {
        for ty in ["pat", "oauth2"] {
            let vars = [("GITLAB_TOKEN_TYPE", ty), ("CI_JOB_TOKEN", "j")];
            let err = with_env(&vars, GitlabAuth::from_env)
                .expect_err(ty)
                .to_string();
            assert!(err.contains("CI_JOB_TOKEN"), "{ty}: {err}");
            assert!(err.contains("GITLAB_TOKEN"), "{ty}: {err}");
        }
    }

    #[test]
    fn env_explicit_non_job_type_with_explicit_token_ignores_ci_job_token() {
        let vars = [
            ("GITLAB_TOKEN", "t"),
            ("GITLAB_TOKEN_TYPE", "pat"),
            ("CI_JOB_TOKEN", "j"),
        ];
        assert_eq!(kind(&vars), GitlabAuthKind::Pat);
    }

    #[test]
    fn env_ci_job_token_alone_selects_job() {
        assert_eq!(kind(&[("CI_JOB_TOKEN", "j")]), GitlabAuthKind::Job);
    }

    #[test]
    fn env_explicit_token_keeps_pat_even_with_ci_job_token() {
        let vars = [("GITLAB_TOKEN", "t"), ("CI_JOB_TOKEN", "j")];
        assert_eq!(kind(&vars), GitlabAuthKind::Pat);
    }

    #[test]
    fn env_blank_token_with_ci_job_token_selects_job() {
        let vars = [("GITLAB_TOKEN", "  "), ("CI_JOB_TOKEN", "j")];
        assert_eq!(kind(&vars), GitlabAuthKind::Job);
    }

    #[test]
    fn env_token_prefers_explicit_over_ci_job_token() {
        let vars = [("GITLAB_TOKEN", "t"), ("CI_JOB_TOKEN", "j")];
        assert_eq!(token(&vars).as_deref(), Some("t"));
        let vars = [("GL_TOKEN", "g"), ("CI_JOB_TOKEN", "j")];
        assert_eq!(token(&vars).as_deref(), Some("g"));
    }

    #[test]
    fn env_token_falls_back_to_ci_job_token() {
        assert_eq!(token(&[("CI_JOB_TOKEN", "j")]).as_deref(), Some("j"));
        let vars = [("GITLAB_TOKEN", "  "), ("CI_JOB_TOKEN", "j")];
        assert_eq!(token(&vars).as_deref(), Some("j"));
    }

    #[test]
    fn env_token_absent_without_any_token_var() {
        assert_eq!(token(&[]), None);
        assert_eq!(token(&[("GITLAB_TOKEN_TYPE", "job")]), None);
    }

    #[test]
    fn work_item_query_targets_group_and_iid() {
        let q = StatusChecker::work_item_query("rigetti/qcs", 146);
        assert_eq!(q["variables"]["fullPath"], "rigetti/qcs");
        assert_eq!(q["variables"]["iid"], "146");
    }

    #[test]
    fn parses_open_and_closed_work_items() {
        let open = json!({"data": {"namespace": {"workItem": {"title": "t", "state": "OPEN"}}}});
        assert_eq!(
            StatusChecker::parse_work_item_response(&open, "g", 1).unwrap(),
            ("OPEN".to_string(), "t".to_string())
        );
        let closed =
            json!({"data": {"namespace": {"workItem": {"title": "t", "state": "CLOSED"}}}});
        assert_eq!(
            StatusChecker::parse_work_item_response(&closed, "g", 1)
                .unwrap()
                .0,
            "CLOSED"
        );
    }

    #[test]
    fn missing_namespace_or_work_item_or_errors_are_not_found() {
        for payload in [
            json!({"data": {"namespace": null}}),
            json!({"data": {"namespace": {"workItem": null}}}),
            json!({"data": null, "errors": [{"message": "boom"}]}),
            json!({"message": "401 Unauthorized"}),
            json!({}),
        ] {
            assert!(
                StatusChecker::parse_work_item_response(&payload, "g", 1).is_err(),
                "{payload}"
            );
        }
    }
}
