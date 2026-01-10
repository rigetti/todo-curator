# TODO Curator

A Rust tool to check TODO comments against issue and merge request status across GitHub and GitLab repositories.

This tool replaces the bash scripts `check-todos-closed-issues.sh` and `check-mr-issues.sh` with a more robust, maintainable Rust implementation.

## Features

- **Multi-platform support**: Works with both GitHub and GitLab
- **Multiple reference formats**: Supports full URLs, rendered format (owner/repo#123), and local references (#123)
- **Two checking modes**:
  - `check-closed`: Find TODO comments referencing closed issues or merged/closed MRs/PRs
  - `check-mr-todos`: Find TODO comments that reference issues closed by the current MR

## Installation

```bash
cd /Users/kstrand/workspace/todo-curator
cargo build --release
cargo install --path .
```

## Prerequisites

You must have both `gh` and `glab` CLI tools installed and authenticated:

- **GitHub CLI (`gh`)**: https://cli.github.com/manual/gh_auth_login
- **GitLab CLI (`glab`)**: https://docs.gitlab.com/editor_extensions/gitlab_cli/#authenticate-with-gitlab

## Usage

### Check for TODOs referencing closed issues/MRs

```bash
todo-curator check-closed [--path <directory>]
```

This command scans the specified directory (default: current directory) for TODO comments that reference closed issues or merged/closed MRs/PRs. If any are found, it exits with status 1.

**Example:**
```bash
cd ~/workspace/my-project
todo-curator check-closed
```

### Check for TODOs that should be removed when MR closes

```bash
todo-curator check-mr-todos [--path <directory>]
```

This command checks if there are TODO comments referencing issues that will be closed by the current GitLab MR. This helps ensure you remove TODOs when their associated issues are resolved.

**Example:**
```bash
cd ~/workspace/my-project
todo-curator check-mr-todos
```

## Supported TODO Reference Formats

### GitLab Issues
- Local: `TODO #123`
- Full URL: `TODO https://gitlab.com/group/subgroup/repo/-/issues/123`
- Without schema: `TODO gitlab.com/group/subgroup/repo/-/issues/123`
- Rendered: `TODO group/subgroup/repo#123`

### GitHub Issues
- Full URL: `TODO https://github.com/owner/repo/issues/123`
- Without schema: `TODO github.com/owner/repo/issues/123`

### GitLab Merge Requests
- Local: `TODO !123`
- Full URL: `TODO https://gitlab.com/group/subgroup/repo/-/merge_requests/123`
- Without schema: `TODO gitlab.com/group/subgroup/repo/-/merge_requests/123`
- Rendered: `TODO group/subgroup/repo!123`

### GitHub Pull Requests
- Full URL: `TODO https://github.com/owner/repo/pull/123`
- Without schema: `TODO github.com/owner/repo/pull/123`

## Exit Codes

- `0`: Success (no issues found)
- `1`: Found TODO comments referencing closed issues/MRs, or authentication failed

## Integration with CI/CD

Add to your GitLab CI pipeline:

```yaml
check-todos:
  stage: test
  script:
    - todo-curator check-closed
  allow_failure: false

check-mr-todos:
  stage: test
  script:
    - todo-curator check-mr-todos
  only:
    - merge_requests
  allow_failure: false
```

## Development

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run -- check-closed

# Format code
cargo fmt

# Lint
cargo clippy
```

## Migrating from Bash Scripts

This tool replaces:
- `check-todos-closed-issues.sh` → `todo-curator check-closed`
- `check-mr-issues.sh` → `todo-curator check-mr-todos`

The Rust implementation provides:
- Better error handling
- Clearer output with colored messages
- More maintainable code
- Faster execution
- Type safety
