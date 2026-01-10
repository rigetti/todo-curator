use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TodoReference {
    GitLabIssue { project: Option<String>, number: u32 },
    GitHubIssue { repo: String, number: u32 },
    GitLabMr { project: Option<String>, number: u32 },
    GitHubPr { repo: String, number: u32 },
}

impl TodoReference {
    pub fn display(&self) -> String {
        match self {
            TodoReference::GitLabIssue { project: None, number } => format!("#{}", number),
            TodoReference::GitLabIssue { project: Some(p), number } => format!("{}#{}", p, number),
            TodoReference::GitHubIssue { repo, number } => format!("{}#{}", repo, number),
            TodoReference::GitLabMr { project: None, number } => format!("!{}", number),
            TodoReference::GitLabMr { project: Some(p), number } => format!("{}!{}", p, number),
            TodoReference::GitHubPr { repo, number } => format!("{}#{}", repo, number),
        }
    }
}

type ExtractorFn = Box<dyn Fn(&regex::Captures) -> Option<TodoReference> + Send + Sync>;

pub struct TodoExtractor {
    patterns: Vec<(Regex, ExtractorFn)>,
}

impl TodoExtractor {
    pub fn new() -> Self {
        let patterns: Vec<(Regex, ExtractorFn)> = vec![
            // Local GitLab issues: TODO #123
            (
                Regex::new(r"\bTODO:? #(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()).map(|num| {
                        TodoReference::GitLabIssue {
                            project: None,
                            number: num,
                        }
                    })
                }),
            ),
            // GitLab issues with full URLs: https://gitlab.com/group/.../repo/-/issues/123
            (
                Regex::new(r"TODO:?.*https?://gitlab\.com/([^/]+(?:/[^/]+)*?)/-/issues/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabIssue {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitLab issues without schema: gitlab.com/group/.../repo/-/issues/123
            (
                Regex::new(r"TODO:?.*gitlab\.com/([^/]+(?:/[^/]+)*?)/-/issues/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabIssue {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitLab issues in rendered format: group/subgroup/repo#123
            (
                Regex::new(r"TODO:?.*\b([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)#(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabIssue {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitHub issues with full URLs: https://github.com/owner/repo/issues/123
            (
                Regex::new(r"TODO:?.*https?://github\.com/([^/]+/[^/]+)/issues/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let repo = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitHubIssue { repo, number })
                }),
            ),
            // GitHub issues without schema: github.com/owner/repo/issues/123
            (
                Regex::new(r"TODO:?.*github\.com/([^/]+/[^/]+)/issues/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let repo = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitHubIssue { repo, number })
                }),
            ),
            // Local GitLab MRs: TODO !123
            (
                Regex::new(r"\bTODO:? !(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()).map(|num| {
                        TodoReference::GitLabMr {
                            project: None,
                            number: num,
                        }
                    })
                }),
            ),
            // GitLab MRs with full URLs: https://gitlab.com/group/.../repo/-/merge_requests/123
            (
                Regex::new(r"TODO:?.*https?://gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabMr {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitLab MRs without schema: gitlab.com/group/.../repo/-/merge_requests/123
            (
                Regex::new(r"TODO:?.*gitlab\.com/([^/]+(?:/[^/]+)*?)/-/merge_requests/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabMr {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitLab MRs in rendered format: group/subgroup/repo!123
            (
                Regex::new(r"TODO:?.*\b([a-zA-Z0-9_-]+(?:/[a-zA-Z0-9_-]+)+)!(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let project = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitLabMr {
                        project: Some(project),
                        number,
                    })
                }),
            ),
            // GitHub PRs with full URLs: https://github.com/owner/repo/pull/123
            (
                Regex::new(r"TODO:?.*https?://github\.com/([^/]+/[^/]+)/pull/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let repo = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitHubPr { repo, number })
                }),
            ),
            // GitHub PRs without schema: github.com/owner/repo/pull/123
            (
                Regex::new(r"TODO:?.*github\.com/([^/]+/[^/]+)/pull/(\d+)").unwrap(),
                Box::new(|caps: &regex::Captures| {
                    let repo = caps.get(1)?.as_str().to_string();
                    let number = caps.get(2)?.as_str().parse::<u32>().ok()?;
                    Some(TodoReference::GitHubPr { repo, number })
                }),
            ),
        ];

        Self { patterns }
    }

    pub fn extract_from_file(&self, path: &Path) -> anyhow::Result<HashSet<TodoReference>> {
        let content = std::fs::read_to_string(path)?;
        Ok(self.extract_from_text(&content))
    }

    pub fn extract_from_text(&self, text: &str) -> HashSet<TodoReference> {
        let mut references = HashSet::new();

        for (pattern, extractor) in &self.patterns {
            for caps in pattern.captures_iter(text) {
                if let Some(reference) = extractor(&caps) {
                    references.insert(reference);
                }
            }
        }

        references
    }

    pub fn extract_from_directory(&self, dir: &Path) -> anyhow::Result<HashSet<TodoReference>> {
        let mut all_references = HashSet::new();

        for entry in WalkDir::new(dir)
            .into_iter()
            .filter_entry(|e| {
                let path = e.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Ok(refs) = self.extract_from_file(entry.path()) {
                    all_references.extend(refs);
                }
            }
        }

        Ok(all_references)
    }
}
