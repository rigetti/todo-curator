# Agent choices: adopt release-plz

Decisions fixed by the user:

- Replace knope entirely.
- Only `GITHUB_TOKEN`: no PAT, no GitHub App.
- No shared `qcs-gha-infrastructure` workflows on the release path: inlined.

## Choices and assumptions

- release-plz 0.4.3 and `release-plz/action` v0.5.148 (commit
  `66b324fd`), the latest releases at the time of writing.
- `git_only = true`: the crate is not on crates.io (`todo-curator` does not
  exist there) although `Cargo.toml` has `publish = ["crates-io"]`.
  release-plz takes the current version from tags and never runs
  `cargo publish`.
  `Cargo.toml` is left alone; release-plz needs no change there.
- Tag `vX.Y.Z` (`git_tag_name`), so history stays continuous.
  Verified locally: baseline resolves to `v0.1.18`; the existing
  `v0.1.19-rc.1` tag is ignored on `main` (not an ancestor).
- `release_always = false`: release only when the release PR is merged.
  release-plz identifies it by its `release-plz-` branch prefix.
- `release_commits` restricts releases to `feat`, `fix`, and breaking commits,
  as knope did.
  Verified locally: a `chore:` commit alone yields "no commit matches".
  Note release-plz only counts commits that touch package files.
- Changelog: `header = ""` and a custom body keep the existing
  `## X.Y.Z (date)` / `### Features` / `### Fixes` format.
  Verified on a throwaway clone: the new entry is prepended above `0.1.18`.
  Only `feat` and `fix` commits are listed; others are skipped.
- Draft-then-publish invariant: `git_release_draft = true`; `publish` job runs
  `gh release upload --clobber` then `gh release edit --draft=false --latest`.
  Not run end to end: it needs a real release.
  Untested assumption: release-plz pushes the tag even for a draft release, so
  `build` can check out `refs/tags/<tag>`.
- No tag-triggered or release-triggered workflow: tags and releases made with
  `GITHUB_TOKEN` start nothing.
  One workflow run (`release.yml`, started by the human merge) does
  tag, build, upload, publish.
- Resume path: `workflow_dispatch` with a `tag` input skips `release-plz
  release` and builds and publishes that tag, since a re-run of `release` finds
  the tag already present and releases nothing.
- Prereleases: release-plz has none.
  `set-version` rewrites the latest changelog heading, so it is unusable
  here.
  Instead `prepare-prerelease.yml` runs `release-plz update --no-changelog` to
  get the next stable version, appends `-<label>.<n>`, commits, and pushes only
  a tag (a commit on no branch), then builds and publishes a prerelease
  draft-then-undraft.
  Reason for not pushing to the branch: the bump would reach `main` with a
  prerelease `Cargo.toml` version, which release-plz would release.
  Tradeoff: the tag's commit is not in branch history.
  The bump/number logic was run locally; the workflow was not dispatched.
- Release PRs from `GITHUB_TOKEN` need `pull_request` runs approved by hand
  and the repo setting for PR creation (see PR description).
- `ci.yml`: only comments updated; `ci-gate` and the `action` job are unchanged.
- Deleted: `knope.toml`, `cli-release.yml`, `prepare-release.yml`.
- Third-party actions pinned to SHAs; release-plz binary for the prerelease job
  downloaded from its GitHub release (no extra action).
- `actionlint` could not be installed here (download denied); workflows were
  parsed with `yq` and their `run` blocks checked with `bash -n`.
