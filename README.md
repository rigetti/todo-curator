# TODO Curator

A Rust tool to check TODO comments against issue and merge request status across GitHub and GitLab repositories.

This tool replaces the bash scripts `check-todos-closed-issues.sh` and `check-mr-issues.sh` with a more robust, maintainable Rust implementation using native API clients.

## Features

- **Multi-platform support**: Works with both GitHub and GitLab via native API clients
- **Multiple reference formats**: Supports full URLs, rendered format (owner/repo#123), and local references (#123)
- **Two checking modes**:
  - `check-closed`: Find TODO comments referencing closed issues or merged/closed MRs/PRs
  - `check-mr-todos`: Find TODO comments that reference issues closed by the current MR
- **No CLI dependencies**: Uses `octocrab` for GitHub and `gitlab` crate for GitLab APIs directly

## Installation

```bash
cd /Users/kstrand/workspace/todo-curator
cargo build --release
cargo install --path .
```

## Prerequisites

You need API tokens for GitHub and GitLab:

### GitHub Authentication

Create a personal access token at https://github.com/settings/tokens with `repo` scope, then set:

```bash
export GITHUB_TOKEN="your_github_token_here"
```

### GitLab Authentication

Create a personal access token at your GitLab instance (Settings → Access Tokens) with `api` scope, then set:

```bash
export GITLAB_TOKEN="your_gitlab_token_here"
# Optional: if using self-hosted GitLab
export GITLAB_URL="https://gitlab.example.com"  # defaults to https://gitlab.com
```

## Usage

### Check for TODOs referencing closed issues/MRs

```bash
todo-curator check-closed [--path <directory>]
```

This command scans the specified directory (default: current directory) for TODO comments that reference closed issues or merged/closed MRs/PRs. If any are found, it exits with status 1.

**Example:**
```bash
export GITHUB_TOKEN="ghp_..."
export GITLAB_TOKEN="glpat-..."

cd ~/workspace/my-project
todo-curator check-closed
```

### Check for TODOs that should be removed when MR closes

```bash
todo-curator check-mr-todos [--path <directory>] --project <gitlab-project-path>
```

This command checks if there are TODO comments referencing issues that will be closed by the current GitLab MR. This helps ensure you remove TODOs when their associated issues are resolved.

The `--project` flag (or `GITLAB_PROJECT` environment variable) specifies the GitLab project path (e.g., `group/subgroup/repo`).

**Example:**
```bash
export GITLAB_TOKEN="glpat-..."
export GITLAB_PROJECT="mygroup/myrepo"

cd ~/workspace/my-project
todo-curator check-mr-todos

# Or specify project directly:
todo-curator check-mr-todos --project mygroup/myrepo
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
  variables:
    GITHUB_TOKEN: $GITHUB_TOKEN  # Set in CI/CD variables
    GITLAB_TOKEN: $GITLAB_TOKEN  # Set in CI/CD variables
  script:
    - todo-curator check-closed
  allow_failure: false

check-mr-todos:
  stage: test
  variables:
    GITLAB_TOKEN: $GITLAB_TOKEN
    GITLAB_PROJECT: $CI_PROJECT_PATH  # Automatically set by GitLab CI
  script:
    - todo-curator check-mr-todos
  only:
    - merge_requests
  allow_failure: false
```

**Note**: Store `GITHUB_TOKEN` and `GITLAB_TOKEN` as masked CI/CD variables in your GitLab project settings.

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
- **Native API clients**: No dependency on `gh` or `glab` CLI tools
- **Better error handling**: Type-safe API interactions
- **Clearer output**: Colored messages for better readability
- **More maintainable**: Pure Rust with well-defined types
- **Faster execution**: Direct API calls without subprocess overhead
- **Type safety**: Compile-time guarantees

### Migration Steps

1. **Install the tool**: `cargo install --path .`
2. **Set up API tokens**: Replace CLI authentication with environment variables
   ```bash
   # Instead of: gh auth login
   export GITHUB_TOKEN="ghp_..."
   
   # Instead of: glab auth login
   export GITLAB_TOKEN="glpat-..."
   ```
3. **Update scripts**: Replace bash script calls with `todo-curator` commands
4. **Update CI/CD**: Add token variables to your CI/CD configuration
