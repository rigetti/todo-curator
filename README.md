# TODO Curator

A Rust tool to check TODO comments against issue and merge request status across GitHub and GitLab repositories.

## Authentication

To check for stale references, you need API tokens for GitHub and GitLab.

If you have `glab` and `gh` installed and authenticated,
you can use `scripts/autoauth-todo-curator.sh` to run `todo-curator` (if it's on the `PATH`),
which will automatically use `glab` and `gh` for authentication.

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

### GitHub and GitLab

- Full URL: `https://gitlab.com/group/subgroup/repo/-/issues/123`
  - Schema is optional (the reference can start with `gitlab.com` or `github.com`).
  - This works for GitLab work-items, merge-requests, and epics, and for GitHub issues and pull-requests.
- Shortened URL: `github.com/owner/repo#7`

## URL-shortening warnings

The "short" versions of references are generally preferred.

When `todo-curator` encounters full URLs,
it prints `TODO references that can be shortened` with a suggested shorter form.

## Integration with CI/CD

Use the reusable workflows and composite actions from
[`rigetti/qcs-gha-infrastructure`](https://github.com/rigetti/qcs-gha-infrastructure).
This repository's own `.github/workflows/` directory is a working example.
