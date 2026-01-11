use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
        repo: String,
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
            TodoReference::GitHubIssue { repo, number, .. } => format!("{}#{}", repo, number),
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
        }
    }

    pub fn source_line(&self) -> &str {
        match self {
            TodoReference::GitLabIssue { source_line, .. } => source_line,
            TodoReference::GitHubIssue { source_line, .. } => source_line,
            TodoReference::GitLabMr { source_line, .. } => source_line,
            TodoReference::GitHubPr { source_line, .. } => source_line,
        }
    }

    pub fn file_path(&self) -> &str {
        match self {
            TodoReference::GitLabIssue { file_path, .. } => file_path,
            TodoReference::GitHubIssue { file_path, .. } => file_path,
            TodoReference::GitLabMr { file_path, .. } => file_path,
            TodoReference::GitHubPr { file_path, .. } => file_path,
        }
    }

    pub fn line_number(&self) -> u64 {
        match self {
            TodoReference::GitLabIssue { line_number, .. } => *line_number,
            TodoReference::GitHubIssue { line_number, .. } => *line_number,
            TodoReference::GitLabMr { line_number, .. } => *line_number,
            TodoReference::GitHubPr { line_number, .. } => *line_number,
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
                            repo,
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
                            repo,
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
                            repo,
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
            .hidden(false) // Don't skip hidden files by default
            .git_ignore(true) // Respect .gitignore
            .git_exclude(true) // Respect .git/info/exclude
            .git_global(true) // Use global gitignore rules
            .standard_filters(true) // Enable standard filters (skips .git directories)
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
