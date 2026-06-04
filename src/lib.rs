pub mod checker;
pub mod todo;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use todo::LintViolationMap;

/// JSON output format for check results
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<checker::ClosedReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_found: Vec<checker::NotFoundReference>,
    #[serde(default, skip_serializing_if = "LintViolationMap::is_empty")]
    pub lint_violations: LintViolationMap,
    pub status: String,
}

impl CheckOutput {
    /// Create a CheckOutput from a CheckResult and lint violations
    pub fn from_result(result: checker::CheckResult, lint_violations: LintViolationMap) -> Self {
        let status = if result.closed.is_empty()
            && result.not_found.is_empty()
            && lint_violations.is_empty()
        {
            "success".to_string()
        } else {
            "failure".to_string()
        };

        Self {
            closed: result.closed,
            not_found: result.not_found,
            lint_violations,
            status,
        }
    }

    /// Create an empty success output
    pub fn empty_success() -> Self {
        Self {
            closed: Vec::new(),
            not_found: Vec::new(),
            lint_violations: LintViolationMap::new(),
            status: "success".to_string(),
        }
    }

    /// Check if there are any errors (closed or not found issues)
    pub fn has_errors(&self) -> bool {
        !self.closed.is_empty() || !self.not_found.is_empty() || !self.lint_violations.is_empty()
    }
}

/// Check closed references in a directory
pub async fn check_closed_references(path: PathBuf) -> Result<CheckOutput> {
    // Detect GitHub and GitLab projects from git origin for local TODO references
    let project_detection = checker::StatusChecker::detect_project(&path);
    match &project_detection {
        checker::ProjectDetection::GitLab(project) => {
            tracing::debug!("Detected GitLab project: {project}");
        }
        checker::ProjectDetection::GitHub(repo) => {
            tracing::debug!("Detected GitHub repo: {repo}");
        }
        checker::ProjectDetection::None => {
            tracing::debug!("No project detected");
        }
    };
    let checker = checker::StatusChecker::with_default_project(project_detection).await?;

    checker.check_auth()?;

    let extractor = todo::TodoExtractor::new();
    let references = extractor.extract_from_directory(&path)?;

    if references.is_empty() {
        return Ok(CheckOutput::empty_success());
    }

    let references_vec: Vec<_> = references.into_iter().collect();
    let result = checker.check_references(&references_vec).await?;

    Ok(CheckOutput::from_result(result, LintViolationMap::new()))
}

/// Check for improperly-formatted TODO comments in a directory
pub fn check_invalid(path: &Path) -> Result<CheckOutput> {
    let linter = todo::TodoLinter::new();
    let lint_violations = linter.lint_directory(path)?;

    if lint_violations.is_empty() {
        Ok(CheckOutput::empty_success())
    } else {
        Ok(CheckOutput {
            closed: Vec::new(),
            not_found: Vec::new(),
            lint_violations,
            status: "failure".to_string(),
        })
    }
}
