#!/usr/bin/env bash

set -euo pipefail

# Check and set GITHUB_TOKEN if not already set
if [[ -z "${GITHUB_TOKEN:-}" ]]; then
    if gh auth status &>/dev/null; then
        export GITHUB_TOKEN=$(gh auth token)
    else
        echo "'gh' is either not working or not authorized." >&2
        echo "See https://cli.github.com/manual/gh_auth_login" >&2
        exit 1
    fi
fi

# Check and set GITLAB_TOKEN if not already set
if [[ -z "${GITLAB_TOKEN:-}" ]]; then
    if glab auth status &>/dev/null; then
        export GITLAB_TOKEN=$(glab config get token --host gitlab.com)
    else
        echo "'glab' is either not working or not authorized." >&2
        echo "See https://docs.gitlab.com/editor_extensions/gitlab_cli/#authenticate-with-gitlab" >&2
        exit 1
    fi
fi

# Run todo-curator with all arguments passed through
exec ~/.cargo/bin/todo-curator check-closed "$@"
