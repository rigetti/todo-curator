pub mod checker;
pub mod todo;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use todo::{ExtractionResult, LintViolationMap};

/// JSON output format for check results
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<checker::ClosedReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_found: Vec<checker::NotFoundReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ReferenceWarning>,
    #[serde(default, skip_serializing_if = "LintViolationMap::is_empty")]
    pub lint_violations: LintViolationMap,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ReferenceWarning {
    pub reference: todo::TodoReference,
    pub original: String,
    pub suggestion: String,
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
            warnings: Vec::new(),
            lint_violations,
            status,
        }
    }

    /// Create an empty success output
    pub fn empty_success() -> Self {
        Self {
            closed: Vec::new(),
            not_found: Vec::new(),
            warnings: Vec::new(),
            lint_violations: LintViolationMap::new(),
            status: "success".to_string(),
        }
    }

    /// Check if there are any errors (closed or not found issues)
    pub fn has_errors(&self) -> bool {
        !self.closed.is_empty() || !self.not_found.is_empty() || !self.lint_violations.is_empty()
    }
}

/// Extract TODO references and lint violations in one pass.
pub fn extract_todos(path: &Path, exclude_file_regexes: &[String]) -> Result<ExtractionResult> {
    let extractor = todo::TodoExtractor::with_exclude_file_regexes(exclude_file_regexes)?;
    extractor.extract_from_directory(path)
}

/// Project a closed-reference check from a precomputed extraction result.
pub async fn check_closed_from_extraction(
    extraction: &ExtractionResult,
    project_detection: &checker::ProjectDetection,
    checker: &checker::StatusChecker,
) -> Result<CheckOutput> {
    match project_detection {
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

    let references = &extraction.references;

    if references.is_empty() {
        return Ok(CheckOutput::empty_success());
    }

    let references_vec: Vec<_> = references.iter().cloned().collect();
    let warnings = collect_url_shortening_warnings(project_detection, &references_vec);
    let result = checker
        .check_references(project_detection, &references_vec)
        .await?;
    let mut output = CheckOutput::from_result(result, LintViolationMap::new());
    output.warnings = warnings;
    Ok(output)
}

/// Project an invalid-pattern check from a precomputed extraction result.
pub fn check_invalid_from_extraction(
    extraction: &ExtractionResult,
    project_detection: &checker::ProjectDetection,
) -> Result<CheckOutput> {
    let lint_violations = extraction.lint_violations.clone();
    let references_vec: Vec<_> = extraction.references.iter().cloned().collect();
    let warnings = collect_url_shortening_warnings(project_detection, &references_vec);

    if lint_violations.is_empty() && warnings.is_empty() {
        Ok(CheckOutput::empty_success())
    } else {
        Ok(CheckOutput {
            closed: Vec::new(),
            not_found: Vec::new(),
            warnings,
            lint_violations,
            status: "failure".to_string(),
        })
    }
}

/// Check closed references in a directory.
pub async fn check_closed_references(
    path: PathBuf,
    project_detection: &checker::ProjectDetection,
    checker: &checker::StatusChecker,
    exclude_file_regexes: &[String],
) -> Result<CheckOutput> {
    let extraction = extract_todos(&path, exclude_file_regexes)?;
    check_closed_from_extraction(&extraction, project_detection, checker).await
}

/// Check for improperly-formatted TODO comments in a directory.
pub fn check_invalid(
    path: &Path,
    project_detection: &checker::ProjectDetection,
    _checker: &checker::StatusChecker,
    exclude_file_regexes: &[String],
) -> Result<CheckOutput> {
    let extraction = extract_todos(path, exclude_file_regexes)?;
    check_invalid_from_extraction(&extraction, project_detection)
}

fn collect_url_shortening_warnings(
    project_detection: &checker::ProjectDetection,
    references: &[todo::TodoReference],
) -> Vec<ReferenceWarning> {
    let mut warnings: Vec<_> = references
        .iter()
        .filter_map(|reference| reference.suggest_simplified_format(project_detection))
        .collect();

    // Deduplicate warnings in case the same URL is parsed more than once from one source line.
    let mut seen: HashSet<(String, u64, String, String)> = HashSet::new();
    warnings.retain(|w| {
        seen.insert((
            w.reference.file_path().to_string(),
            w.reference.line_number(),
            w.original.clone(),
            w.suggestion.clone(),
        ))
    });

    warnings.sort_by(|a, b| {
        a.reference
            .file_path()
            .cmp(b.reference.file_path())
            .then(a.reference.line_number().cmp(&b.reference.line_number()))
            .then(a.original.cmp(&b.original))
    });

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_and_suggests_short_form_for_same_gitlab_project_issue_url() {
        let refs = vec![todo::TodoReference {
            kind: todo::TodoReferenceKind::GitLabIssue {
                project: Some("foo/bar/baz".to_string()),
                number: 7,
            },
            source_line: "// TODO https://gitlab.com/foo/bar/baz/-/issues/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 10,
            original_reference: "https://gitlab.com/foo/bar/baz/-/issues/7".to_string(),
            recommended_format: Some("foo/bar/baz#7".to_string()),
        }];

        let warnings = collect_url_shortening_warnings(
            &checker::ProjectDetection::GitLab("foo/bar/baz".to_string()),
            &refs,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].suggestion, "#7");
    }

    #[test]
    fn warns_and_suggests_project_scoped_form_for_external_gitlab_issue_url() {
        let refs = vec![todo::TodoReference {
            kind: todo::TodoReferenceKind::GitLabIssue {
                project: Some("foo/bar/baz".to_string()),
                number: 7,
            },
            source_line: "// TODO https://gitlab.com/foo/bar/baz/-/work_items/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 20,
            original_reference: "https://gitlab.com/foo/bar/baz/-/work_items/7".to_string(),
            recommended_format: Some("foo/bar/baz#7".to_string()),
        }];

        let warnings = collect_url_shortening_warnings(
            &checker::ProjectDetection::GitLab("other/team/project".to_string()),
            &refs,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].suggestion, "foo/bar/baz#7");
    }

    #[test]
    fn warns_and_suggests_github_prefixed_form_for_same_github_repo_issue_url() {
        let refs = vec![todo::TodoReference {
            kind: todo::TodoReferenceKind::GitHubIssue {
                repo: Some("owner/repo".to_string()),
                number: 7,
            },
            source_line: "// TODO https://github.com/owner/repo/issues/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 10,
            original_reference: "https://github.com/owner/repo/issues/7".to_string(),
            recommended_format: Some("github.com/owner/repo#7".to_string()),
        }];

        let warnings = collect_url_shortening_warnings(
            &checker::ProjectDetection::GitHub("owner/repo".to_string()),
            &refs,
        );

        assert_eq!(warnings.len(), 1);
        // Must use github.com/ prefix even for same repo: bare #N parses as GitLab.
        assert_eq!(warnings[0].suggestion, "github.com/owner/repo#7");
    }

    #[test]
    fn warns_and_suggests_github_prefixed_form_for_cross_repo_github_issue_url() {
        let refs = vec![todo::TodoReference {
            kind: todo::TodoReferenceKind::GitHubIssue {
                repo: Some("owner/repo".to_string()),
                number: 7,
            },
            source_line: "// TODO https://github.com/owner/repo/issues/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 10,
            original_reference: "https://github.com/owner/repo/issues/7".to_string(),
            recommended_format: Some("github.com/owner/repo#7".to_string()),
        }];

        let warnings = collect_url_shortening_warnings(
            &checker::ProjectDetection::GitHub("other/project".to_string()),
            &refs,
        );

        assert_eq!(warnings.len(), 1);
        // Must use github.com/ prefix: bare owner/repo#N would be parsed as GitLab.
        assert_eq!(warnings[0].suggestion, "github.com/owner/repo#7");
    }
}
