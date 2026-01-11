pub mod checker;
pub mod todo;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// JSON output format for check results
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckOutput {
    pub closed: Vec<checker::ClosedReference>,
    pub not_found: Vec<checker::NotFoundReference>,
    pub status: String,
}

impl CheckOutput {
    /// Create a CheckOutput from a CheckResult
    pub fn from_result(result: checker::CheckResult) -> Self {
        let status = if result.closed.is_empty() && result.not_found.is_empty() {
            "success".to_string()
        } else {
            "failure".to_string()
        };

        Self {
            closed: result.closed,
            not_found: result.not_found,
            status,
        }
    }

    /// Create an empty success output
    pub fn empty_success() -> Self {
        Self {
            closed: Vec::new(),
            not_found: Vec::new(),
            status: "success".to_string(),
        }
    }

    /// Check if there are any errors (closed or not found issues)
    pub fn has_errors(&self) -> bool {
        !self.closed.is_empty() || !self.not_found.is_empty()
    }
}

/// Check closed references in a directory
pub async fn check_closed_references(path: PathBuf) -> Result<CheckOutput> {
    // Detect GitLab project from git origin for local TODO references
    let gitlab_project = checker::StatusChecker::detect_gitlab_project(&path);
    tracing::debug!("GitLab project: {gitlab_project:?}");
    let checker = checker::StatusChecker::with_default_project(gitlab_project).await?;

    checker.check_auth()?;

    let extractor = todo::TodoExtractor::new();
    let references = extractor.extract_from_directory(&path)?;

    if references.is_empty() {
        return Ok(CheckOutput::empty_success());
    }

    let references_vec: Vec<_> = references.into_iter().collect();
    let result = checker.check_references(&references_vec).await?;

    Ok(CheckOutput::from_result(result))
}
