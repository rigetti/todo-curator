# Agent choices: adopt release-plz

Decisions fixed by the user:

- Replace knope entirely.
- Only `GITHUB_TOKEN`: no PAT, no GitHub App.
- No shared `qcs-gha-infrastructure` workflows on the release path: inlined.
- Releases are deliberate: they go through a release PR, opened only by a
  manual dispatch, and merging it releases.
  Nothing releases on an ordinary push.
- Prerelease keeps a local bump commit on top of the branch head, so
  `Cargo.toml` in the tagged tree matches the binary's version.
  Only the tag is pushed; nothing lands on the branch.
- `--latest` is never passed.
  GitHub's default selection of the "latest" release is relied on.

## Workflows

- `prepare-release.yml` (dispatch on `main`): `release-plz release-pr`.
  The PR is opened and updated with `GITHUB_TOKEN`, so its `pull_request` runs
  wait in `action_required` until a maintainer approves them;
  every push by release-plz needs a new approval.
  This depends on the repository requiring approval for first-time
  contributors; GitHub's documentation says `GITHUB_TOKEN` events start no runs.
  Close and reopen is the fallback
  (`ci.yml` uses default `pull_request` types, which include `reopened`).
  The strict ruleset means a PR behind `main` needs fresh checks after any
  later push to `main`: "Update branch" (a user push) or a new dispatch plus
  approval.
- `release.yml` (`pull_request: closed`, plus `workflow_dispatch` on `main`
  for recovery): releases the version in `Cargo.toml`.
  - Runs for a merged PR into `main`, from this repository, labeled
    `release` (release-plz's `pr_labels`).
    Anyone with triage access can add the label to another PR; merging it
    then releases the current `Cargo.toml` version, which normally already
    has a tag, so nothing happens.
  - Checks out `merge_commit_sha`, the PR's commit on `main` for any merge
    method, so commits merged later are never part of the release.
  - The merge is done by a person, so the run is not held for approval,
    unlike the release PR's other `pull_request` runs.
  - No separate tag validation: `cargo metadata` rejects a `Cargo.toml`
    version that is not valid semver.
  - Tag missing: `release-plz release` creates tag and draft.
    Tag present: reused, and release-plz is skipped.
    Tag present without a release: fails, asking to delete the tag and
    dispatch again.
  - No early refusal of an already published version (`publish` refuses a
    non-draft, after the builds), no refusal of a prerelease version in
    `Cargo.toml`, and no check that an existing tag is an ancestor of `main`.
  - `build-release.yml` (`workflow_call`, shared with the prerelease, called
    by local path so it runs at the caller's commit): `build` from the tag;
    `publish` requires a draft, then `gh release upload --clobber` and
    `gh release edit --draft=false`, which keeps the prerelease flag.
  - Re-running after a failure is the recovery path (no `tag` input).
- `prepare-prerelease.yml` (dispatch on a non-`main` branch): the next version
  from `release-plz update --no-changelog`, plus `-<label>.<n>` with `n`
  computed from existing tags.
  - Label validated: `^[A-Za-z][0-9A-Za-z-]*$`; `release-plz set-version`
    rejects a version that is not valid semver, before anything is pushed.
  - A `concurrency` group per repository and branch (no cancel) keeps two
    dispatches from computing the same number.
  - Refuses when `Cargo.toml` has no release tag
    (release PR merged but not released, or a release PR branch).
  - The bump commit subject carries `[run <run id>]`; a re-run of the same
    dispatch finds its tag by that and reuses it, never computing `n+1`.
  - The version is bumped with `release-plz set-version`.
    It also renames the newest `CHANGELOG.md` heading, which is harmless:
    that commit exists only under the prerelease tag.
  - The draft prerelease is created in the first job, with `--prerelease`;
    an existing release is reused.

## release-plz configuration

- release-plz 0.4.3 and `release-plz/action` v0.5.148 (commit `66b324fd`).
  `prepare-prerelease.yml` needs `release-plz update`, which the action does
  not run, so it installs the same version with `taiki-e/install-action`
  (v2.89.1, the oldest release whose manifest has release-plz 0.4.3;
  it verifies the download's SHA-256).
- `publish = false` in `Cargo.toml` (the crate is not on crates.io).
  `git_only = true` is needed: without it, release-plz skips a
  `publish = false` package.
  With it, the version comes from tags and nothing is published.
- Defaults for everything else: no `release_commits`, no `[changelog]`, no
  `git_tag_name`.
  Only settings the manual `GITHUB_TOKEN` flow needs are set: `git_only`,
  `git_release_draft`, `git_release_type`, `release_always`, `pr_labels`.
- Tag: the default for a single package, `v{{ version }}`.
  The baseline is the highest tag matching `^v(\d+\.\d+\.\d+)$`, so rc tags
  such as `v0.1.19-rc.1` are ignored.
- `release_always = true`: releases are manual, so release the version in
  `Cargo.toml` that has no tag.
  `false` would require the newest commit on `main` to be the release PR merge.
- What counts as a change (default `release_commits`): every commit that
  touches a file `cargo package` would include, whatever its type.
  This repo sets no `include`/`exclude`, so `.github/`, `release-plz.toml`,
  and lockfile changes count: a history of only `chore(deps)` or workflow
  commits gives a patch bump.
  In 0.x, a `feat` gives a patch bump and a breaking change (`!`) a minor bump.
- Changelog: release-plz's default format (Keep a Changelog headings such as
  `### Added`, `### Fixed`, `### Other`; `## [X.Y.Z](compare-url) - date`).
  Older knope entries (`## X.Y.Z (date)`) are left as they are.
  New entries are prepended under the header and above the old entries.
  `CHANGELOG.md` is seeded with the default header and an `## [Unreleased]`
  line; without them, release-plz leaves no blank line before the first old
  entry.

## Deferred

- Restricting the release workflows to an environment limited to `main`.

## Open

- The release PR's checks need a maintainer's approval on every release-plz
  push; there is no token-free way around it.
