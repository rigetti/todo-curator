use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::{DirEntry, WalkBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{checker::ProjectDetection, ReferenceWarning};

/// Walk source files in a directory, respecting `.gitignore` and standard filters.
/// Yields only regular files (skips directories, symlinks, errors).
fn walk_source_files(dir: &Path) -> impl Iterator<Item = DirEntry> {
    let mut builder = WalkBuilder::new(dir);
    builder.standard_filters(true);
    let ci_file = dir.join(".gitlab-ci.yml");
    if ci_file.exists() {
        builder.add(ci_file);
    }
    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_file()))
}

/// A lint violation found in a TODO comment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LintViolation {
    pub source_line: String,
    pub file_path: String,
    pub line_number: u64,
}

/// A list of TODO-like comment lines that do not match accepted TODO reference syntax.
pub type LintViolations = Vec<LintViolation>;

/// Result of TODO extraction, including parsed references and syntax violations.
#[derive(Debug, Default)]
pub struct ExtractionResult {
    pub references: HashSet<TodoReference>,
    pub lint_violations: LintViolations,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TodoReferenceKind {
    GitLabIssue {
        project: Option<String>,
        number: u32,
    },
    GitHubIssueOrPr {
        repo: Option<String>,
        number: u32,
    },
    GitLabMr {
        project: Option<String>,
        number: u32,
    },
    GitHubPr {
        repo: String,
        number: u32,
    },
    GitLabEpic {
        group: Option<String>,
        number: u32,
    },
    /// A group-level GitLab work item (not an epic or a project issue).
    GitLabWorkItem {
        group: String,
        number: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TodoReference {
    pub kind: TodoReferenceKind,
    pub source_line: String,
    pub file_path: String,
    pub line_number: u64,
    /// String token as matched from the TODO reference segment.
    pub original_reference: String,
    /// Suggested shorter format for URL-form references; `None` if already in preferred form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_format: Option<String>,
}

impl TodoReference {
    #[must_use]
    pub fn new(
        kind: TodoReferenceKind,
        source_line: &str,
        file_path: &str,
        line_number: u64,
        original_reference: &str,
        recommended_format: Option<String>,
    ) -> Self {
        Self {
            kind,
            source_line: source_line.trim().to_string(),
            file_path: file_path.to_string(),
            line_number,
            original_reference: original_reference.to_string(),
            recommended_format,
        }
    }

    #[must_use]
    pub fn display(&self) -> String {
        let (scope, sigil, number) = match &self.kind {
            TodoReferenceKind::GitLabIssue {
                project, number, ..
            } => (project.as_deref(), '#', number),
            TodoReferenceKind::GitHubIssueOrPr { repo, number, .. } => {
                (repo.as_deref(), '#', number)
            }
            TodoReferenceKind::GitLabMr {
                project, number, ..
            } => (project.as_deref(), '!', number),
            TodoReferenceKind::GitHubPr { repo, number, .. } => (Some(repo.as_str()), '#', number),
            TodoReferenceKind::GitLabEpic { group, number, .. } => (group.as_deref(), '&', number),
            TodoReferenceKind::GitLabWorkItem { group, number } => {
                (Some(group.as_str()), '#', number)
            }
        };
        format!("{}{sigil}{number}", scope.unwrap_or_default())
    }

    #[must_use]
    pub fn source_line(&self) -> &str {
        &self.source_line
    }

    #[must_use]
    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    #[must_use]
    pub const fn line_number(&self) -> u64 {
        self.line_number
    }

    #[must_use]
    pub fn suggest_simplified_format(
        &self,
        project_detection: &ProjectDetection,
    ) -> Option<ReferenceWarning> {
        let base_suggestion = self.recommended_format.as_deref()?;

        let suggestion = match &self.kind {
            TodoReferenceKind::GitLabIssue {
                project: Some(project),
                number,
                ..
            } => {
                if project_detection.matches_gitlab(project) {
                    format!("#{number}")
                } else {
                    base_suggestion.to_string()
                }
            }
            TodoReferenceKind::GitLabMr {
                project: Some(project),
                number,
                ..
            } => {
                if project_detection.matches_gitlab(project) {
                    format!("!{number}")
                } else {
                    base_suggestion.to_string()
                }
            }
            TodoReferenceKind::GitLabEpic {
                group: Some(group),
                number,
                ..
            } => {
                if project_detection.parent_group_matches(group) {
                    format!("&{number}")
                } else {
                    base_suggestion.to_string()
                }
            }
            _ => base_suggestion.to_string(),
        };

        Some(ReferenceWarning {
            reference: self.clone(),
            original: self.original_reference.clone(),
            suggestion,
        })
    }
}

type ExtractorFn =
    Box<dyn Fn(&regex::Captures<'_>, &str, &str, u64) -> Option<TodoReference> + Send + Sync>;

type Pattern = (Regex, ExtractorFn);

pub struct TodoExtractor {
    todo_ref_pattern: Regex,
    patterns: Vec<(Regex, ExtractorFn)>,
    exclude_file_regex: Option<Regex>,
}

fn local_gitlab_issue_pattern() -> Pattern {
    // Local GitLab issues: #123
    // TODO(#6): support local GitHub issues and PRs
    (
        Regex::new(r"^#(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let number = caps.get(1)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabIssue {
                        project: None,
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn gitlab_group_work_item_url_pattern() -> Pattern {
    // GitLab group-level work items with optional schema:
    // https://gitlab.com/groups/group/.../path/-/work_items/123
    // gitlab.com/groups/group/.../path/-/work_items/123
    // NOTE: must come BEFORE the project issue pattern, which would otherwise
    // read `groups/<group>` as a project path.
    (
        Regex::new(r"^(?:https?://)?gitlab\.com/groups/([^/]+(?:/[^/]+)*?)/-/work_items/(\d+)$")
            .unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let group = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                // `a/b#N` parses as a project issue, so only a single-segment group
                // has a short form that round-trips.
                let recommended = group
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    .then(|| format!("{group}#{number}"));
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabWorkItem { group, number },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    recommended,
                ))
            },
        ),
    )
}

fn gitlab_issue_url_pattern() -> Pattern {
    // GitLab issues/work items with optional schema:
    // https://gitlab.com/group/.../repo/-/issues/123
    // gitlab.com/group/.../repo/-/issues/123
    // https://gitlab.com/group/.../repo/-/work_items/123
    // gitlab.com/group/.../repo/-/work_items/123
    (
        Regex::new(
            r"^(?:https?://)?gitlab\.com/([^/]+(?:/[^/]+)*?)/-/(?:issues|work_items)/(\d+)$",
        )
        .unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let project = caps.get(1)?.as_str().to_string();
                // `groups/...` URLs are group-level; never treat them as projects.
                if project == "groups" || project.starts_with("groups/") {
                    return None;
                }
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabIssue {
                        project: Some(project.clone()),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    Some(format!("{project}#{number}")),
                ))
            },
        ),
    )
}

fn github_issue_url_pattern() -> Pattern {
    // GitHub issues with optional schema:
    // https://github.com/owner/repo/issues/123
    // github.com/owner/repo/issues/123
    (
        Regex::new(r"^(?:https?://)?github\.com/([^/]+/[^/]+)/issues/(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let repo = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitHubIssueOrPr {
                        repo: Some(repo.clone()),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    // Always use github.com/ prefix: bare owner/repo#N or #N would
                    // be misinterpreted as GitLab refs by the parser.
                    // TODO(#6): support GitHub "short" refs
                    Some(format!("github.com/{repo}#{number}")),
                ))
            },
        ),
    )
}

fn github_issue_shorthand_pattern() -> Pattern {
    // GitHub issues in shorthand format: github.com/owner/repo#123
    (
        Regex::new(r"^github\.com/([^/]+/[^/]+)#(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let repo = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitHubIssueOrPr {
                        repo: Some(repo),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn gitlab_issue_rendered_pattern() -> Pattern {
    // GitLab issues in rendered format: group/subgroup/repo#123
    // NOTE: This must come AFTER GitHub patterns to avoid misclassifying github.com URLs
    // TODO(#6): support GitHub issues and PRs
    (
        Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)#(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let project = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabIssue {
                        project: Some(project),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn gitlab_group_work_item_rendered_pattern() -> Pattern {
    // GitLab group-level work items in rendered format: group#123
    // Only a single path segment; `group/repo#123` is a project issue (above).
    (
        Regex::new(r"^([a-zA-Z0-9_-]+)#(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let group = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabWorkItem { group, number },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn local_gitlab_mr_pattern() -> Pattern {
    // Local GitLab MRs: !123
    (
        Regex::new(r"^!(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let number = caps.get(1)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabMr {
                        project: None,
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn gitlab_mr_url_pattern() -> Pattern {
    // GitLab MRs with optional schema:
    // https://gitlab.com/group/.../repo/-/merge_requests/123
    // gitlab.com/group/.../repo/-/merge_requests/123
    (
        Regex::new(r"^(?:https?://)?gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)$")
            .unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let project = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabMr {
                        project: Some(project.clone()),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    Some(format!("{project}!{number}")),
                ))
            },
        ),
    )
}

fn gitlab_mr_rendered_pattern() -> Pattern {
    // GitLab MRs in rendered format: group/subgroup/repo!123
    (
        Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)!(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let project = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabMr {
                        project: Some(project),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn github_pr_url_pattern() -> Pattern {
    // GitHub PRs with optional schema:
    // https://github.com/owner/repo/pull/123
    // github.com/owner/repo/pull/123
    (
        Regex::new(r"^(?:https?://)?github\.com/([^/]+/[^/]+)/pull/(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let repo = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitHubPr { repo, number },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn github_pr_bang_pattern() -> Pattern {
    // GitHub PRs with GitLab-style `!` syntax; the github.com host makes it unambiguous:
    // https://github.com/owner/repo!123
    // github.com/owner/repo!123
    (
        Regex::new(r"^(?:https?://)?github\.com/([^/]+/[^/!]+)!(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let repo = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitHubPr { repo, number },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn local_gitlab_epic_pattern() -> Pattern {
    // Local GitLab epics: &123
    (
        Regex::new(r"^&(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let number = caps.get(1)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabEpic {
                        group: None,
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

fn gitlab_epic_url_pattern() -> Pattern {
    // GitLab epics with optional schema:
    // https://gitlab.com/groups/group/.../path/-/epics/123
    // gitlab.com/groups/group/.../path/-/epics/123
    (
        Regex::new(r"^(?:https?://)?gitlab\.com/groups/([^/]+(?:/[^/]+)*?)/-/epics/(\d+)$")
            .unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let group = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabEpic {
                        group: Some(group.clone()),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    Some(format!("{group}&{number}")),
                ))
            },
        ),
    )
}

fn gitlab_epic_rendered_pattern() -> Pattern {
    // GitLab epics in rendered format: group/subgroup/path&123
    (
        Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)&(\d+)$").unwrap(),
        Box::new(
            |caps: &regex::Captures<'_>, line: &str, file_path: &str, line_number: u64| {
                let group = caps.get(1)?.as_str().to_string();
                let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                Some(TodoReference::new(
                    TodoReferenceKind::GitLabEpic {
                        group: Some(group),
                        number,
                    },
                    line,
                    file_path,
                    line_number,
                    caps.get(0)?.as_str(),
                    None,
                ))
            },
        ),
    )
}

impl Default for TodoExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoExtractor {
    #[must_use]
    pub fn new() -> Self {
        Self::with_exclude_file_regex("").expect("empty string is a valid regex")
    }

    pub fn with_exclude_file_regex(exclude_file_regex: &str) -> anyhow::Result<Self> {
        // Extract TODO reference candidates in two forms:
        // 1) TODO <single-ref>:
        // 2) TODO(<multiple-refs>) or TODO (<multiple-refs>)
        //
        // This is intentionally applied with captures_iter so multiple TODOs on one line are
        // each processed independently, e.g. "TODO (#7) TODO #18: TODO(#1,#2)".
        let todo_ref_pattern = Regex::new(
            r"(?x)
                \bTODO\b # TODO on a word boundary
                (?:
                    \s+
                    (?P<single_ref>
                        (?:https?://)?  # refs can start with a URL schema
                        [^\s():]+       # refs can't contain whitespace or parentheses, or colons after the URL schema
                    )
                    (?:
                        :?                # may be followed by a colon
                        (?:
                            \s+               # 'TODO <ref>:' with trailing text
                            |$                # 'TODO <ref>:' at end of line
                        )
                    )
                    |
                    \s*\(
                    (?P<multiple_refs>[^)]+?)
                    \)
                )
            ",
        )
        .unwrap();

        let patterns: Vec<Pattern> = vec![
            local_gitlab_issue_pattern(),
            gitlab_group_work_item_url_pattern(),
            gitlab_issue_url_pattern(),
            github_issue_url_pattern(),
            github_issue_shorthand_pattern(),
            gitlab_issue_rendered_pattern(),
            gitlab_group_work_item_rendered_pattern(),
            local_gitlab_mr_pattern(),
            gitlab_mr_url_pattern(),
            gitlab_mr_rendered_pattern(),
            github_pr_url_pattern(),
            github_pr_bang_pattern(),
            local_gitlab_epic_pattern(),
            gitlab_epic_url_pattern(),
            gitlab_epic_rendered_pattern(),
        ];

        let exclude_file_regex = if exclude_file_regex.trim().is_empty() {
            None
        } else {
            Some(Regex::new(exclude_file_regex).map_err(|e| {
                anyhow::anyhow!("Invalid --exclude-file-regex value '{exclude_file_regex}': {e}")
            })?)
        };

        Ok(Self {
            todo_ref_pattern,
            patterns,
            exclude_file_regex,
        })
    }

    fn extract_token_reference(
        &self,
        token: &str,
        line: &str,
        file_path: &str,
        line_number: u64,
    ) -> Option<TodoReference> {
        let token = token.trim();
        if token.is_empty() {
            return None;
        }

        for (pattern, extractor) in &self.patterns {
            if let Some(caps) = pattern.captures(token) {
                if let Some(reference) = extractor(&caps, line, file_path, line_number) {
                    return Some(reference);
                }
            }
        }

        None
    }

    /// Records references found on `line`, or a lint violation if it has none.
    fn process_line(
        &self,
        line: &str,
        file_path: &str,
        lnum: u64,
        refs: &Mutex<HashSet<TodoReference>>,
        viols: &Mutex<LintViolations>,
    ) {
        let mut line_has_match = false;

        for todo_captures in self.todo_ref_pattern.captures_iter(line) {
            if let Some(single_ref) = todo_captures.name("single_ref") {
                let single_ref = single_ref.as_str();
                if single_ref.trim() == "performance" {
                    line_has_match = true;
                    continue;
                }

                if let Some(reference) =
                    self.extract_token_reference(single_ref, line, file_path, lnum)
                {
                    line_has_match = true;
                    refs.lock().unwrap().insert(reference);
                }
            }

            if let Some(multiple_refs) = todo_captures.name("multiple_refs") {
                for token in multiple_refs.as_str().split(',').map(str::trim) {
                    if token.is_empty() {
                        continue;
                    }

                    if token.trim() == "performance" {
                        line_has_match = true;
                        continue;
                    }

                    if let Some(reference) =
                        self.extract_token_reference(token, line, file_path, lnum)
                    {
                        line_has_match = true;
                        refs.lock().unwrap().insert(reference);
                    }
                }
            }
        }

        if !line_has_match {
            viols.lock().unwrap().push(LintViolation {
                source_line: line.trim().to_string(),
                file_path: file_path.to_string(),
                line_number: lnum,
            });
        }
    }

    pub fn extract_from_directory(&self, dir: &Path) -> anyhow::Result<ExtractionResult> {
        tracing::debug!(
            "Extracting TODO references from directory: {}",
            dir.display()
        );
        let all_references = Arc::new(Mutex::new(HashSet::new()));
        let lint_violations: Arc<Mutex<LintViolations>> = Arc::new(Mutex::new(Vec::new()));

        // Build a combined regex pattern that matches all tracked TODO-like markers.
        // Searcher only invokes the callback on matching lines,
        // so this must include all lint-triggering keywords (e.g. FIXME), not just TODO.
        // Lowercase `todo` and `temp` are ignored unless they are clearly part of a single-line code comment,
        // since common patterns such as `temp` as a variable and the `todo!` Rust macro would otherwise trigger false positives.
        let todo_pattern = r"\bTODO\b|(?i:\b(?:xxx|fixme|tbd|mvp)\b)|\bTEMP\b|(?:#|//).*(?:\btemp\b|\btodo(?:\b$|\b[^-]))";
        let matcher = RegexMatcher::new(todo_pattern)?;

        for entry in walk_source_files(dir) {
            let path = entry.path();
            tracing::debug!("Processing file: {}", path.display());
            let relative_file_path_str = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            if self
                .exclude_file_regex
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(&relative_file_path_str))
            {
                continue;
            }

            let refs = Arc::clone(&all_references);
            let viols = Arc::clone(&lint_violations);
            let file_path_str = path.to_string_lossy().to_string();

            // Use grep-searcher to efficiently search the file
            let mut searcher = Searcher::new();
            let result = searcher.search_path(
                &matcher,
                path,
                UTF8(|lnum, line| {
                    self.process_line(line, &file_path_str, lnum, &refs, &viols);
                    Ok(true)
                }),
            );

            // Log errors for individual files (e.g., binary files, permission issues)
            if let Err(e) = result {
                tracing::warn!("Failed to search file {}: {}", path.display(), e);
            }
        }

        let references = Arc::try_unwrap(all_references).map_or_else(
            |arc| arc.lock().unwrap().clone(),
            |mutex| mutex.into_inner().unwrap(),
        );
        let lint_violations = Arc::try_unwrap(lint_violations).map_or_else(
            |arc| arc.lock().unwrap().clone(),
            |mutex| mutex.into_inner().unwrap(),
        );
        let mut deduped_lint_violations: HashMap<(String, u64, String), LintViolation> =
            HashMap::new();
        for violation in lint_violations {
            deduped_lint_violations
                .entry((
                    violation.file_path.clone(),
                    violation.line_number,
                    violation.source_line.clone(),
                ))
                .or_insert(violation);
        }
        let mut lint_violations: Vec<LintViolation> =
            deduped_lint_violations.into_values().collect();
        lint_violations.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line_number.cmp(&b.line_number))
                .then(a.source_line.cmp(&b.source_line))
        });
        tracing::debug!(directory=%dir.display(), ?references, ?lint_violations, "Extracted TODO references");
        Ok(ExtractionResult {
            references,
            lint_violations,
        })
    }
}
