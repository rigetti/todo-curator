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
pub enum TodoReference {
    GitLabIssue {
        project: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitHubIssue {
        repo: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitLabMr {
        project: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitHubPr {
        repo: String,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitLabEpic {
        group: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
}

impl TodoReference {
    pub fn display(&self) -> String {
        match self {
            TodoReference::GitLabIssue {
                project: None,
                number,
                ..
            } => format!("#{}", number),
            TodoReference::GitLabIssue {
                project: Some(p),
                number,
                ..
            } => format!("{}#{}", p, number),
            TodoReference::GitHubIssue {
                repo: Some(repo),
                number,
                ..
            } => format!("{}#{}", repo, number),
            TodoReference::GitHubIssue {
                repo: None, number, ..
            } => format!("#{}", number),
            TodoReference::GitLabMr {
                project: None,
                number,
                ..
            } => format!("!{}", number),
            TodoReference::GitLabMr {
                project: Some(p),
                number,
                ..
            } => format!("{}!{}", p, number),
            TodoReference::GitHubPr { repo, number, .. } => format!("{}#{}", repo, number),
            TodoReference::GitLabEpic {
                group: None,
                number,
                ..
            } => format!("&{}", number),
            TodoReference::GitLabEpic {
                group: Some(g),
                number,
                ..
            } => format!("{}&{}", g, number),
        }
    }

    pub fn source_line(&self) -> &str {
        match self {
            TodoReference::GitLabIssue { source_line, .. } => source_line,
            TodoReference::GitHubIssue { source_line, .. } => source_line,
            TodoReference::GitLabMr { source_line, .. } => source_line,
            TodoReference::GitHubPr { source_line, .. } => source_line,
            TodoReference::GitLabEpic { source_line, .. } => source_line,
        }
    }

    pub fn file_path(&self) -> &str {
        match self {
            TodoReference::GitLabIssue { file_path, .. } => file_path,
            TodoReference::GitHubIssue { file_path, .. } => file_path,
            TodoReference::GitLabMr { file_path, .. } => file_path,
            TodoReference::GitHubPr { file_path, .. } => file_path,
            TodoReference::GitLabEpic { file_path, .. } => file_path,
        }
    }

    pub fn line_number(&self) -> u64 {
        match self {
            TodoReference::GitLabIssue { line_number, .. } => *line_number,
            TodoReference::GitHubIssue { line_number, .. } => *line_number,
            TodoReference::GitLabMr { line_number, .. } => *line_number,
            TodoReference::GitHubPr { line_number, .. } => *line_number,
            TodoReference::GitLabEpic { line_number, .. } => *line_number,
        }
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
                        [^\s()]+          # single ref token up to whitespace
                    )
                    (?:
                        \s+               # TODO <ref> with trailing text
                        |$                 # TODO <ref> at end of line
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
            (
                Regex::new(r"^#(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
                        caps.get(1)
                            .and_then(|m| m.as_str().parse::<u32>().ok())
                            .map(|num| TodoReference::GitLabIssue {
                                project: None,
                                number: num,
                                source_line: line.trim().to_string(),
                                file_path: file_path.to_string(),
                                line_number,
                            })
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
                        Some(TodoReference::GitLabIssue {
                            project: Some(project),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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
                        Some(TodoReference::GitHubIssue {
                            repo: Some(repo),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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
                        Some(TodoReference::GitHubIssue {
                            repo: Some(repo),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
                    },
                ),
            ),
            // GitLab issues in rendered format: group/subgroup/repo#123
            // NOTE: This must come AFTER GitHub patterns to avoid misclassifying github.com URLs
            (
                Regex::new(r"^([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)#(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
                        let project = caps.get(1)?.as_str().to_string();
                        let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                        Some(TodoReference::GitLabIssue {
                            project: Some(project),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
                    },
                ),
            ),
            // Local GitLab MRs: !123
            (
                Regex::new(r"^!(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
                        caps.get(1)
                            .and_then(|m| m.as_str().parse::<u32>().ok())
                            .map(|num| TodoReference::GitLabMr {
                                project: None,
                                number: num,
                                source_line: line.trim().to_string(),
                                file_path: file_path.to_string(),
                                line_number,
                            })
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
                        Some(TodoReference::GitLabMr {
                            project: Some(project),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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
                        Some(TodoReference::GitLabMr {
                            project: Some(project),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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
                        Some(TodoReference::GitHubPr {
                            repo,
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
                    },
                ),
            ),
            // Local GitLab epics: &123
            (
                Regex::new(r"^&(\d+)$").unwrap(),
                Box::new(
                    |caps: &regex::Captures, line: &str, file_path: &str, line_number: u64| {
                        caps.get(1)
                            .and_then(|m| m.as_str().parse::<u32>().ok())
                            .map(|num| TodoReference::GitLabEpic {
                                group: None,
                                number: num,
                                source_line: line.trim().to_string(),
                                file_path: file_path.to_string(),
                                line_number,
                            })
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
                        Some(TodoReference::GitLabEpic {
                            group: Some(group),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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
                        Some(TodoReference::GitLabEpic {
                            group: Some(group),
                            number,
                            source_line: line.trim().to_string(),
                            file_path: file_path.to_string(),
                            line_number,
                        })
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

                    for todo_caps in self.todo_ref_pattern.captures_iter(line) {
                        if let Some(single_ref) = todo_caps.name("single_ref") {
                            let single_ref = single_ref.as_str().trim_end_matches(':');
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

                        if let Some(multiple_refs) = todo_caps.name("multiple_refs") {
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
