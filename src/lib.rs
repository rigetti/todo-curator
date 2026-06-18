pub mod checker;
pub mod todo;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use todo::LintViolationMap;

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

/// Check closed references in a directory
pub async fn check_closed_references(
    path: PathBuf,
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

    let extractor = todo::TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&path)?;
    let references = extraction.references;

    if references.is_empty() {
        let mut output = CheckOutput::empty_success();
        output.lint_violations = extraction.lint_violations;
        if !output.lint_violations.is_empty() {
            output.status = "failure".to_string();
        }
        return Ok(output);
    }

    let references_vec: Vec<_> = references.into_iter().collect();
    let warnings = collect_url_shortening_warnings(project_detection, &references_vec);
    let result = checker
        .check_references(project_detection, &references_vec)
        .await?;
    let mut output = CheckOutput::from_result(result, extraction.lint_violations);
    output.warnings = warnings;
    Ok(output)
}

/// Check for improperly-formatted TODO comments in a directory
pub fn check_invalid(
    path: &Path,
    _project_detection: &checker::ProjectDetection,
    _checker: &checker::StatusChecker,
    exclude_file_regexes: &[String],
) -> Result<CheckOutput> {
    let linter = todo::TodoLinter::with_exclude_file_regexes(exclude_file_regexes)?;
    let lint_violations = linter.lint_directory(path)?;

    if lint_violations.is_empty() {
        Ok(CheckOutput::empty_success())
    } else {
        Ok(CheckOutput {
            closed: Vec::new(),
            not_found: Vec::new(),
            warnings: Vec::new(),
            lint_violations,
            status: "failure".to_string(),
        })
    }
}

fn collect_url_shortening_warnings(
    project_detection: &checker::ProjectDetection,
    references: &[todo::TodoReference],
) -> Vec<ReferenceWarning> {
    let mut warnings = Vec::new();

    for reference in references {
        match reference {
            todo::TodoReference::GitLabIssue {
                project: Some(project),
                number,
                ..
            } => {
                let issue_url = format!("https://gitlab.com/{project}/-/issues/{number}");
                let work_item_url = format!("https://gitlab.com/{project}/-/work_items/{number}");
                if let Some(original) =
                    find_original_url(reference.source_line(), &[&issue_url, &work_item_url])
                {
                    let suggestion = match project_detection {
                        checker::ProjectDetection::GitLab(current) if current == project => {
                            format!("#{number}")
                        }
                        _ => format!("{project}#{number}"),
                    };
                    warnings.push(ReferenceWarning {
                        reference: reference.clone(),
                        original,
                        suggestion,
                    });
                }
            }
            todo::TodoReference::GitHubIssue {
                repo: Some(repo),
                number,
                ..
            } => {
                let issue_url = format!("https://github.com/{repo}/issues/{number}");
                if let Some(original) = find_original_url(reference.source_line(), &[&issue_url]) {
                    let suggestion = match project_detection {
                        checker::ProjectDetection::GitHub(current) if current == repo => {
                            format!("#{number}")
                        }
                        _ => format!("{repo}#{number}"),
                    };
                    warnings.push(ReferenceWarning {
                        reference: reference.clone(),
                        original,
                        suggestion,
                    });
                }
            }
            todo::TodoReference::GitLabMr {
                project: Some(project),
                number,
                ..
            } => {
                let mr_url = format!("https://gitlab.com/{project}/-/merge_requests/{number}");
                if let Some(original) = find_original_url(reference.source_line(), &[&mr_url]) {
                    let suggestion = match project_detection {
                        checker::ProjectDetection::GitLab(current) if current == project => {
                            format!("!{number}")
                        }
                        _ => format!("{project}!{number}"),
                    };
                    warnings.push(ReferenceWarning {
                        reference: reference.clone(),
                        original,
                        suggestion,
                    });
                }
            }
            todo::TodoReference::GitLabEpic {
                group: Some(group),
                number,
                ..
            } => {
                let epic_url = format!("https://gitlab.com/groups/{group}/-/epics/{number}");
                if let Some(original) = find_original_url(reference.source_line(), &[&epic_url]) {
                    let suggestion = match project_detection {
                        checker::ProjectDetection::GitLab(project)
                            if default_gitlab_group(project).as_deref() == Some(group.as_str()) =>
                        {
                            format!("&{number}")
                        }
                        _ => format!("{group}&{number}"),
                    };
                    warnings.push(ReferenceWarning {
                        reference: reference.clone(),
                        original,
                        suggestion,
                    });
                }
            }
            _ => {}
        }
    }

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

fn find_original_url(source_line: &str, candidates: &[&str]) -> Option<String> {
    for candidate in candidates {
        if source_line.contains(candidate) {
            return Some((*candidate).to_string());
        }
    }
    None
}

fn default_gitlab_group(project_path: &str) -> Option<String> {
    let parts: Vec<&str> = project_path.split('/').collect();
    if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("/"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_and_suggests_short_form_for_same_gitlab_project_issue_url() {
        let refs = vec![todo::TodoReference::GitLabIssue {
            project: Some("foo/bar/baz".to_string()),
            number: 7,
            source_line: "// TODO https://gitlab.com/foo/bar/baz/-/issues/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 10,
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
        let refs = vec![todo::TodoReference::GitLabIssue {
            project: Some("foo/bar/baz".to_string()),
            number: 7,
            source_line: "// TODO https://gitlab.com/foo/bar/baz/-/work_items/7".to_string(),
            file_path: "sample.rs".to_string(),
            line_number: 20,
        }];

        let warnings = collect_url_shortening_warnings(
            &checker::ProjectDetection::GitLab("other/team/project".to_string()),
            &refs,
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].suggestion, "foo/bar/baz#7");
    }
}
