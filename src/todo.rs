use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::{DirEntry, WalkBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
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
        .filter_map(|result| result.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
}

/// Categories of lint violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LintCategory {
    NonMergeable,
    MvpComment,
    IncorrectSyntax,
    Uncapitalized,
}

impl LintCategory {
    /// Header text for this category when printing violations.
    pub fn header(&self) -> &'static str {
        match self {
            Self::NonMergeable => "Non-mergeable TODOs:",
            Self::MvpComment => "Non-mergeable TODOs:",
            Self::IncorrectSyntax => "Improperly-formatted TODO comments:",
            Self::Uncapitalized => "Improperly-formatted TODO comments:",
        }
    }

    /// Optional hint printed once beneath the header.
    pub fn header_hint(&self) -> Option<&'static str> {
        match self {
            Self::IncorrectSyntax | Self::Uncapitalized => Some(
                r#"use `TODO(<ref>)` or `TODO <ref>:`, where `<ref>` is `[repo]#<ticket>`, `[repo]!<merge-request>`, `[group]&<epic>`, or `performance`"#,
            ),
            _ => None,
        }
    }
}

impl fmt::Display for LintCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.header())
    }
}

/// A lint violation found in a TODO comment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LintViolation {
    pub category: LintCategory,
    pub source_line: String,
    pub file_path: String,
    pub line_number: u64,
}

/// A map of lint violations grouped by category.
pub type LintViolationMap = HashMap<LintCategory, Vec<LintViolation>>;

/// Result of TODO extraction, including parsed references and syntax violations.
#[derive(Debug, Default)]
pub struct ExtractionResult {
    pub references: HashSet<TodoReference>,
    pub lint_violations: LintViolationMap,
}

struct LintRule {
    category: LintCategory,
    pattern: Regex,
    exclude_pattern: Option<Regex>,
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

    pub fn display(&self) -> String {
        match &self.kind {
            TodoReferenceKind::GitLabIssue {
                project: None,
                number,
                ..
            } => format!("#{}", number),
            TodoReferenceKind::GitLabIssue {
                project: Some(p),
                number,
                ..
            } => format!("{}#{}", p, number),
            TodoReferenceKind::GitHubIssueOrPr {
                repo: Some(repo),
                number,
                ..
            } => format!("{}#{}", repo, number),
            TodoReferenceKind::GitHubIssueOrPr {
                repo: None, number, ..
            } => format!("#{}", number),
            TodoReferenceKind::GitLabMr {
                project: None,
                number,
                ..
            } => format!("!{}", number),
            TodoReferenceKind::GitLabMr {
                project: Some(p),
                number,
                ..
            } => format!("{}!{}", p, number),
            TodoReferenceKind::GitHubPr { repo, number, .. } => format!("{}#{}", repo, number),
            TodoReferenceKind::GitLabEpic {
                group: None,
                number,
                ..
            } => format!("&{}", number),
            TodoReferenceKind::GitLabEpic {
                group: Some(g),
                number,
                ..
            } => format!("{}&{}", g, number),
        }
    }

    pub fn source_line(&self) -> &str {
        &self.source_line
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    pub fn line_number(&self) -> u64 {
        self.line_number
    }

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
    Box<dyn Fn(&regex::Captures, &str, &str, u64) -> Option<TodoReference> + Send + Sync>;

pub struct TodoExtractor {
    todo_ref_pattern: Regex,
    patterns: Vec<(Regex, ExtractorFn)>,
    lint_rules: Vec<LintRule>,
    exclude_file_regexes: Vec<Regex>,
}

impl Default for TodoExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoExtractor {
    pub fn new() -> Self {
        Self::with_exclude_file_regexes(&[]).expect("default extractor regexes should compile")
    }

    pub fn with_exclude_file_regexes(exclude_file_regexes: &[String]) -> anyhow::Result<Self> {
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

        let patterns: Vec<(Regex, ExtractorFn)> = vec![
            // Local GitLab issues: #123
            // TODO(#6): support local GitHub issues and PRs
            (
                Regex::new(r"^#(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
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
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
                        let project = caps.get(1)?.as_str().to_string();
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
            ),
            // GitHub issues with optional schema:
            // https://github.com/owner/repo/issues/123
            // github.com/owner/repo/issues/123
            (
                Regex::new(r"^(?:https?://)?github\.com/([^/]+/[^/]+)/issues/(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitHub issues in shorthand format: github.com/owner/repo#123
            (
                Regex::new(r"^github\.com/([^/]+/[^/]+)#(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitLab issues in rendered format: group/subgroup/repo#123
            // NOTE: This must come AFTER GitHub patterns to avoid misclassifying github.com URLs
            // TODO(#6): support GitHub issues and PRs
            (
                Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)#(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // Local GitLab MRs: !123
            (
                Regex::new(r"^!(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitLab MRs with optional schema:
            // https://gitlab.com/group/.../repo/-/merge_requests/123
            // gitlab.com/group/.../repo/-/merge_requests/123
            (
                Regex::new(r"^(?:https?://)?gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)$")
                    .unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitLab MRs in rendered format: group/subgroup/repo!123
            (
                Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)!(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitHub PRs with optional schema:
            // https://github.com/owner/repo/pull/123
            // github.com/owner/repo/pull/123
            (
                Regex::new(r"^(?:https?://)?github\.com/([^/]+/[^/]+)/pull/(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // Local GitLab epics: &123
            (
                Regex::new(r"^&(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitLab epics with optional schema:
            // https://gitlab.com/groups/group/.../path/-/epics/123
            // gitlab.com/groups/group/.../path/-/epics/123
            (
                Regex::new(r"^(?:https?://)?gitlab\.com/groups/([^/]+(?:/[^/]+)*?)/-/epics/(\d+)$")
                    .unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
            // GitLab epics in rendered format: group/subgroup/path&123
            (
                Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)&(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
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
            ),
        ];

        let lint_rules = vec![
            LintRule {
                category: LintCategory::NonMergeable,
                pattern: Regex::new(r"\b(XXX|FIXME|TEMP|TBD)\b").unwrap(),
                exclude_pattern: None,
            },
            LintRule {
                category: LintCategory::MvpComment,
                pattern: Regex::new(r"\b(MVP)\b").unwrap(),
                exclude_pattern: None,
            },
            LintRule {
                category: LintCategory::Uncapitalized,
                pattern: Regex::new(r"(#|//).*\b(todo|xxx|fixme|temp|tbd)\b").unwrap(),
                exclude_pattern: Some(Regex::new(r"(todo|xxx|fixme|temp|tbd)-").unwrap()),
            },
        ];

        let exclude_file_regexes = exclude_file_regexes
            .iter()
            .map(|pattern| {
                Regex::new(pattern).map_err(|e| {
                    anyhow::anyhow!("Invalid --exclude-file-regex value '{pattern}': {e}")
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(Self {
            todo_ref_pattern,
            patterns,
            lint_rules,
            exclude_file_regexes,
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

    pub fn extract_from_directory(&self, dir: &Path) -> anyhow::Result<ExtractionResult> {
        tracing::debug!(
            "Extracting TODO references from directory: {}",
            dir.display()
        );
        let all_references = Arc::new(Mutex::new(HashSet::new()));
        let lint_violations: Arc<Mutex<LintViolationMap>> = Arc::new(Mutex::new(HashMap::new()));
        let todo_presence_pattern = Regex::new(r"\bTODO\b")?;

        // Build a combined regex pattern that matches any TODO comment
        let todo_pattern = r"\bTODO:?";
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
                .exclude_file_regexes
                .iter()
                .any(|pattern| pattern.is_match(&relative_file_path_str))
            {
                continue;
            }

            let refs = all_references.clone();
            let viols = lint_violations.clone();
            let file_path_str = path.to_string_lossy().to_string();

            // Use grep-searcher to efficiently search the file
            let mut searcher = Searcher::new();
            let result = searcher.search_path(
                &matcher,
                path,
                UTF8(|lnum, line| {
                    let mut line_has_match = false;

                    for rule in &self.lint_rules {
                        if rule.pattern.is_match(line) {
                            if let Some(ref exclude) = rule.exclude_pattern {
                                if exclude.is_match(line) {
                                    continue;
                                }
                            }
                            viols
                                .lock()
                                .unwrap()
                                .entry(rule.category)
                                .or_default()
                                .push(LintViolation {
                                    category: rule.category,
                                    source_line: line.trim().to_string(),
                                    file_path: file_path_str.clone(),
                                    line_number: lnum,
                                });
                        }
                    }

                    for todo_captures in self.todo_ref_pattern.captures_iter(line) {
                        if let Some(single_ref) = todo_captures.name("single_ref") {
                            let single_ref = single_ref.as_str();
                            if single_ref.trim() == "performance" {
                                line_has_match = true;
                                continue;
                            }

                            if let Some(reference) =
                                self.extract_token_reference(single_ref, line, &file_path_str, lnum)
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
                                    self.extract_token_reference(token, line, &file_path_str, lnum)
                                {
                                    line_has_match = true;
                                    refs.lock().unwrap().insert(reference);
                                }
                            }
                        }
                    }

                    if todo_presence_pattern.is_match(line) && !line_has_match {
                        viols
                            .lock()
                            .unwrap()
                            .entry(LintCategory::IncorrectSyntax)
                            .or_default()
                            .push(LintViolation {
                                category: LintCategory::IncorrectSyntax,
                                source_line: line.trim().to_string(),
                                file_path: file_path_str.clone(),
                                line_number: lnum,
                            });
                    }

                    Ok(true)
                }),
            );

            // Log errors for individual files (e.g., binary files, permission issues)
            if let Err(e) = result {
                tracing::warn!("Failed to search file {}: {}", path.display(), e);
            }
        }

        let references = Arc::try_unwrap(all_references)
            .map(|mutex| mutex.into_inner().unwrap())
            .unwrap_or_else(|arc| arc.lock().unwrap().clone());
        let lint_violations = Arc::try_unwrap(lint_violations)
            .map(|mutex| mutex.into_inner().unwrap())
            .unwrap_or_else(|arc| arc.lock().unwrap().clone());
        tracing::debug!(directory=%dir.display(), ?references, ?lint_violations, "Extracted TODO references");
        Ok(ExtractionResult {
            references,
            lint_violations,
        })
    }
}
