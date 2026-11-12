# TODO Curator

A Rust tool to check TODO comments against issue and merge request status across GitHub and GitLab repositories.

## Authentication

To check for stale references, you need API tokens for GitHub and GitLab.

If you have `glab` and `gh` installed and authenticated,
you can use `scripts/autoauth-todo-curator.sh` to run `todo-curator` (if it's on the `PATH`),
which will automatically use `glab` and `gh` for authentication.

GitLab auth scheme is selected by `GITLAB_TOKEN_TYPE` (or `GL_TOKEN_TYPE`):

- `pat` (default): personal access token, sent as `PRIVATE-TOKEN`.
- `oauth2` (aliases `oauth`, `bearer`): OAuth2 token, sent as `Authorization: Bearer`.
- `job`: GitLab CI job token, sent as `JOB-TOKEN`.

Unknown values are an error.
Inside GitLab CI, set `GITLAB_TOKEN=$CI_JOB_TOKEN` and `GITLAB_TOKEN_TYPE=job`.
If neither `GITLAB_TOKEN` nor `GL_TOKEN` is set and `GITLAB_TOKEN_TYPE` is unset,
`CI_JOB_TOKEN` is used automatically in `job` mode.
A job token can only read what the CI job token allowlist permits.

For local development, you can source `auth-setup.sh` before running the API integration tests.
The tests for each forge are not run unless corresponding feature flag,
`test-integration-github` or `test-integration-gitlab`, is enabled.

## Supported TODO Formats

In each pattern, `#ref` must be a valid reference (see supported formats below).

- single reference: `TODO(#ref)`
  - a space after `TODO` is permitted.
- alternate: `TODO #ref:`
- multiple references: `TODO(#ref, #ref, #ref)`
  - multiple references are not supported without parentheses.

## Supported reference formats

### GitLab only

- work-item in the current repo: `#123`
- work-item in another repo: `group/subgroup/repo#123`
- merge request for the current repo: `!123`
- merge request in another repo: `group/subgroup/repo!123`
- epic under the current project's supgroup: `&123`
  - this uses the "most specific" path, i.e. for `group/sub1/sub2/project`,
    the short-form epic reference only checks for epics in `group/sub1/sub2`.
- full path to an epic: `group/subgroup&123`
- group-level work-item: `group#123`
  - only a single-segment group is supported in this short form,
    since `group/subgroup#123` means a work-item in the project `group/subgroup`.
  - the full URL form `gitlab.com/groups/group/subgroup/-/work_items/123` supports nested groups.
  - checked via the GitLab GraphQL API.

### GitHub and GitLab

- Full URL: `https://gitlab.com/group/subgroup/repo/-/issues/123`
  - Schema is optional (the reference can start with `gitlab.com` or `github.com`).
  - This works for GitLab work-items (project and group level), merge-requests, and epics, and for GitHub issues and pull-requests.
- Shortened URL: `github.com/owner/repo#7`

## URL-shortening warnings

The "short" versions of references are generally preferred.

When `todo-curator` encounters full URLs,
it prints `TODO references that can be shortened` with a suggested shorter form.

## Shell completions

`todo-curator --usage-spec` prints a [usage](https://usage.jdx.dev) spec.
With the `usage` CLI installed, generate completions from it, e.g. for zsh:

```sh
usage generate completion zsh todo-curator --usage-cmd "todo-curator --usage-spec" --install
```

Supported shells include bash, zsh, fish, PowerShell, and nushell.

## Integration with CI/CD

### GitHub Actions

This repository ships an action that installs `todo-curator` from its GitHub
releases with [ubi](https://github.com/houseabsolute/ubi) and runs a check:

```yaml
- uses: rigetti/todo-curator@v0.1.12
  with:
    github-token: ${{ secrets.GITHUB_TOKEN }}
    gitlab-token: ${{ secrets.MY_GITLAB_TOKEN }}  # only for GitLab references
    command: check-all        # or check-closed / check-invalid /
                              # check-mr-todos / validate-auth
    args: --format json       # optional
    path: .                   # optional
    exclude-file-regex: ""    # optional; a single regex, not a list
```

The `version` to install defaults to the ref the action was called with, so
`@v0.1.12` above installs v0.1.12 and there is only one version to pin.

`secrets.GITHUB_TOKEN` covers both the download and same-repository references.
References to other repositories, or to GitLab, need a token with access to
them. `check-invalid` needs no credentials at all. A token is also what keeps
the install off GitHub's unauthenticated API rate limit, which can be as low as
60 requests per hour per IP.

If a repository's own tests or docs contain TODO-shaped text, set
`exclude-file-regex` or the checks will flag them.

## Releasing

Releases are always started by hand, with [release-plz](https://release-plz.dev)
configured in `release-plz.toml`.
Nothing releases automatically on a push.

1. Run the `Prepare release` workflow on `main`.
   It opens or updates a release PR (version bump and changelog) from the
   Conventional Commits (`feat:`, `fix:`, or breaking `!`) since the last
   release.
2. Close and reopen the release PR.
   Events caused by `GITHUB_TOKEN` start no workflow runs (except
   `workflow_dispatch` and `repository_dispatch`), so the PR has no checks
   until it is reopened; `ci.yml` runs on `reopened`.
   The required-checks ruleset is strict, so repeat this after any later push to
   `main`.
   Runs held in `action_required` were observed for a push to an existing PR
   branch (PR #54), not for a newly opened release PR; approve them if they
   appear.
   Then review and merge.
3. Run the `Release` workflow on `main` immediately after merging.
   It tags, creates a draft release, builds the binaries, attaches them, and
   only then publishes the release.
   release-plz tags the release PR's commit only for a merge-commit merge.
   For a squash or rebase merge it tags whatever `main` is at dispatch time, so
   anything that landed after the merge is released too and is missing from
   `CHANGELOG.md`, which is generated when the PR is opened or updated.
   `Release` refuses a prerelease version in `Cargo.toml`.
   If it fails midway, run it again: it reuses the tag and the draft.

For a build of unmerged work, run `Prepare prerelease` on a branch.
It pushes a tag only; nothing lands on the branch.
