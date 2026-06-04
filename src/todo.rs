use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A lint violation found in a TODO comment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LintViolation {
    pub rule: String,
    pub hint: String,
    pub source_line: String,
    pub file_path: String,
    pub line_number: u64,
}

struct LintRule {
    name: String,
    pattern: Regex,
    hint: String,
    file_exclude: Regex,
}

/// Linter that detects improperly-formatted TODO comments.
pub struct TodoLinter {
    rules: Vec<LintRule>,
}

impl TodoLinter {
    pub fn new() -> Self {
        let rules = vec![
            LintRule {
                name: "non-mergeable TODOs".to_string(),
                pattern: Regex::new(r"\b(XXX|FIXME|TEMP|TBD)\b").unwrap(),
                hint: "XXX, FIXME, TEMP, and TBD must be resolved prior to merge".to_string(),
                file_exclude: Regex::new(r"(^|/)docs/todo-comments\.md$|mermaid.*\.js$").unwrap(),
            },
            LintRule {
                name: "MVP comments".to_string(),
                pattern: Regex::new(r"\b(MVP)\b").unwrap(),
                hint: "Comments should not refer to 'MVP'".to_string(),
                file_exclude: Regex::new(r"(^|/)docs/todo-comments\.md$|mermaid.*\.js$").unwrap(),
            },
            LintRule {
                name: "TODOs with incorrect syntax".to_string(),
                pattern: Regex::new(
                    r"\bTODO(?! (github\.com/[-\w]+/[-\w]+|([-\w]+/)+[-\w]+)?[#!&]\d+\b| performance)...",
                )
                .unwrap(),
                hint: r#"use "TODO [repo]#<ticket>", "TODO [repo]!<merge-request>", "TODO [group]&<epic>", or "TODO performance" for TODO comments"#.to_string(),
                file_exclude: Regex::new(
                    r"(^|/)docs/todo-comments\.md$|(^|/)docs/repo-rules-and-guidelines\.md$|(^|/)vendor/|scripts/check-.*issues\.sh$|mermaid.*\.js$",
                )
                .unwrap(),
            },
            LintRule {
                name: "uncapitalized TODO patterns".to_string(),
                pattern: Regex::new(r"(#|//).*\b(todo|xxx|fixme|temp|tbd)\b(?!-curator)").unwrap(),
                hint: r#"use "TODO [repo]#<ticket>", "TODO [repo]!<merge-request>", "TODO [group]&<epic>", or "TODO performance" for TODO comments"#.to_string(),
                file_exclude: Regex::new(r"(^|/)docs/todo-comments\.md$|mermaid.*\.js$").unwrap(),
            },
        ];

        Self { rules }
    }

    pub fn lint_directory(&self, dir: &Path) -> anyhow::Result<Vec<LintViolation>> {
        let violations = Arc::new(Mutex::new(Vec::new()));

        // Match any line that could contain a violation
        let combined_pattern =
            r"\b(XXX|FIXME|TEMP|TBD|MVP|TODO)\b|(#|//).*\b(todo|xxx|fixme|temp|tbd)\b";
        let matcher = RegexMatcher::new(combined_pattern)?;

        let walker = WalkBuilder::new(dir)
            .standard_filters(true)
            .add(".gitlab-ci.yml")
            .build();

        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            let file_path_str = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            let viols = violations.clone();
            let mut searcher = Searcher::new();
            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|lnum, line| {
                    for rule in &self.rules {
                        if rule.file_exclude.is_match(&file_path_str) {
                            continue;
                        }
                        if rule.pattern.is_match(line) {
                            viols.lock().unwrap().push(LintViolation {
                                rule: rule.name.clone(),
                                hint: rule.hint.clone(),
                                source_line: line.trim().to_string(),
                                file_path: path.to_string_lossy().to_string(),
                                line_number: lnum,
                            });
                        }
                    }
                    Ok(true)
                }),
            );
        }

        let result = Arc::try_unwrap(violations)
            .map(|mutex| mutex.into_inner().unwrap())
            .unwrap_or_else(|arc| arc.lock().unwrap().clone());
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[expect(clippy::enum_variant_names, reason = "GitHub and GitLab are distinct")]
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
    patterns: Vec<(Regex, ExtractorFn)>,
}

impl TodoExtractor {
    pub fn new() -> Self {
        let patterns: Vec<(Regex, ExtractorFn)> = vec![
            // Local GitLab issues: TODO #123
            (
                Regex::new(r"\bTODO:? #(\d+)").unwrap(),
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
            // GitLab issues with full URLs: https://gitlab.com/group/.../repo/-/issues/123
            (
                Regex::new(r"TODO:?.*https?://gitlab\.com/([^/]+(?:/[^/]+)*?)/-/issues/(\d+)")
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
            // GitLab issues without schema: gitlab.com/group/.../repo/-/issues/123
            (
                Regex::new(r"TODO:?.*gitlab\.com/([^/]+(?:/[^/]+)*?)/-/issues/(\d+)").unwrap(),
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
            // GitHub issues with full URLs: https://github.com/owner/repo/issues/123
            (
                Regex::new(r"TODO:? https?://github\.com/([^/]+/[^/]+)/issues/(\d+)").unwrap(),
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
            // GitHub issues without schema: github.com/owner/repo/issues/123
            (
                Regex::new(r"TODO:? github\.com/([^/]+/[^/]+)/issues/(\d+)").unwrap(),
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
                Regex::new(r"TODO:? github\.com/([^/]+/[^/]+)#(\d+)").unwrap(),
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
                Regex::new(r"TODO:? \b([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)#(\d+)").unwrap(),
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
            // Local GitLab MRs: TODO !123
            (
                Regex::new(r"\bTODO:? !(\d+)").unwrap(),
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
            // GitLab MRs with full URLs: https://gitlab.com/group/.../repo/-/merge_requests/123
            (
                Regex::new(
                    r"TODO:? https?://gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)",
                )
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
            // GitLab MRs without schema: gitlab.com/group/.../repo/-/merge_requests/123
            (
                Regex::new(r"TODO:? gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)")
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
                Regex::new(r"TODO:? \b([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)!(\d+)").unwrap(),
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
            // GitHub PRs with full URLs: https://github.com/owner/repo/pull/123
            (
                Regex::new(r"TODO:? https?://github\.com/([^/]+/[^/]+)/pull/(\d+)").unwrap(),
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
            // GitHub PRs without schema: github.com/owner/repo/pull/123
            (
                Regex::new(r"TODO:? github\.com/([^/]+/[^/]+)/pull/(\d+)").unwrap(),
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
            // Local GitLab epics: TODO &123
            (
                Regex::new(r"\bTODO:? &(\d+)").unwrap(),
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
            // GitLab epics with full URLs: https://gitlab.com/groups/group/.../path/-/epics/123
            (
                Regex::new(r"TODO:? https?://gitlab\.com/groups/([^/]+(?:/[^/]+)*?)/-/epics/(\d+)")
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
            // GitLab epics without schema: gitlab.com/groups/group/.../path/-/epics/123
            (
                Regex::new(r"TODO:? gitlab\.com/groups/([^/]+(?:/[^/]+)*?)/-/epics/(\d+)").unwrap(),
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
                Regex::new(r"TODO:? \b([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)&(\d+)").unwrap(),
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

        Self { patterns }
    }

    pub fn extract_from_directory(&self, dir: &Path) -> anyhow::Result<HashSet<TodoReference>> {
        tracing::debug!(
            "Extracting TODO references from directory: {}",
            dir.display()
        );
        let all_references = Arc::new(Mutex::new(HashSet::new()));

        // Build a combined regex pattern that matches any TODO comment
        let todo_pattern = r"\bTODO:?";
        let matcher = RegexMatcher::new(todo_pattern)?;

        // Use ignore crate to walk directory respecting .gitignore
        let walker = WalkBuilder::new(dir)
            .standard_filters(true) // Enable standard filters
            .add(".gitlab-ci.yml")
            .build();

        for result in walker {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            tracing::debug!("Processing file: {}", path.display());
            let refs = all_references.clone();
            let file_path_str = path.to_string_lossy().to_string();

            // Use grep-searcher to efficiently search the file
            let mut searcher = Searcher::new();
            let result = searcher.search_path(
                &matcher,
                path,
                UTF8(|lnum, line| {
                    // Extract references from the matched line with file path and line number
                    for (pattern, extractor) in &self.patterns {
                        for caps in pattern.captures_iter(line) {
                            if let Some(reference) = extractor(&caps, line, &file_path_str, lnum) {
                                refs.lock().unwrap().insert(reference);
                            }
                        }
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
        tracing::debug!(directory=%dir.display(), ?references, "Extracted TODO references");
        Ok(references)
    }
}
