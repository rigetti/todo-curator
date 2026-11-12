# Agent choices: adopt release-plz

Decisions fixed by the user:

- Replace knope entirely.
- Only `GITHUB_TOKEN`: no PAT, no GitHub App.
- No shared `qcs-gha-infrastructure` workflows on the release path: inlined.
- Releases are always manual: every release workflow is `workflow_dispatch`
  only; nothing triggers on a push.
- Prerelease keeps a local bump commit on top of the branch head, so
  `Cargo.toml` in the tagged tree matches the binary's version.
  Only the tag is pushed; nothing lands on the branch.
- `--latest` is never passed.
  GitHub's default selection of the "latest" release is relied on.

## Workflows

- `prepare-release.yml` (dispatch on `main`): `release-plz release-pr`.
  The PR's `pull_request` runs wait in `action_required` (PR made by
  `GITHUB_TOKEN`) and go stale after later pushes to `main`; close and reopen
  the PR, per the release-plz docs.
- `release.yml` (dispatch on `main`, after merging the release PR): releases
  the version in `Cargo.toml`.
  - Validates the tag against a strict SemVer regex (no leading zeros).
  - Refuses a version that is already a published (non-draft) release.
  - Tag missing: `release-plz release` creates tag and draft.
    Tag present: reused, and it must be an ancestor of `main`.
    Draft missing: created with `gh release create --draft --verify-tag` and
    the changelog section as notes.
  - `build` from the tag; `publish` requires a draft, then
    `gh release upload --clobber` and `gh release edit --draft=false`.
  - Re-running after a failure is the recovery path (no `tag` input).
- `prepare-prerelease.yml` (dispatch on a non-`main` branch): the next version
  from `release-plz update --no-changelog`, plus `-<label>.<n>` with `n`
  computed from existing tags.
  - Label validated: `^[A-Za-z][0-9A-Za-z-]*$`.
  - The bump commit subject carries `[run <run id>]`; a re-run of the same
    dispatch finds its tag by that and reuses it, never computing `n+1`.
  - Publish is re-runnable: existing draft reused, already published is a
    no-op.

## release-plz configuration

- release-plz 0.4.3 and `release-plz/action` v0.5.148 (commit `66b324fd`),
  the latest releases at the time of writing.
- `publish = false` in `Cargo.toml` (the crate is not on crates.io).
  `git_only = true` is kept: verified locally that without it release-plz
  silently skips a `publish = false` package.
  With it, the version comes from tags and nothing is published.
- Tag `vX.Y.Z` (`git_tag_name`).
  The baseline is the highest tag matching `^v(\d+\.\d+\.\d+)$` over all
  tags, so rc tags such as `v0.1.19-rc.1` are ignored; verified: the baseline
  resolves to `v0.1.18`.
- `release_always = true`: releases are manual, so release the version in
  `Cargo.toml` that has no tag.
  `false` would require the newest commit on `main` to be the release PR merge.
- `release_commits`: `feat`, `fix`, `type!:`, or a `BREAKING CHANGE:` footer.
  Verified locally: `refactor!:` and a `chore:` with a footer both bump to
  0.2.0; a lone `chore:` does not.
  release-plz only counts commits that touch package files.
- Changelog: keeps the `## X.Y.Z (date)` / `### Features` / `### Fixes`
  format.
  `header = "# Changelog\n"` (the file gets that header, added in this PR) and
  `trim = false` produce clean spacing; an empty header leaves a blank first
  line.
  Verified on a throwaway clone: entries prepend correctly, the header is not
  duplicated.
  Breaking commits with `!` are listed under "Breaking changes"; a footer-only
  breaking commit still appears under its type heading (the `body` parser did
  not take effect).
- Release notes: `parse-changelog` extracts the section for the new heading
  format (verified for `0.2.0` and `0.1.18`).
  Assumption: release-plz uses the same parser for the release body.
  A dry-run `release-plz release` ran without error but does not print the body.

## Not verified

- No workflow was dispatched; no release, draft, or prerelease was created.
- Untested: release-plz pushes the tag for a draft release, the
  upload/undraft sequence on a real draft, the prerelease install and tag push.
- `actionlint` is unavailable here (download denied); workflows parsed with
  `yq`, `run` blocks checked with `bash -n` and `shellcheck -S warning`.

## Open

- `Prepare release` runs each need a manual approval (`action_required`); no
  token-free way around it.
