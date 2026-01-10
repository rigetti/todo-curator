#!/usr/bin/env bash

set -euo pipefail

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

# Set GITLAB_PROJECT if not already set (use CI_PROJECT_PATH if available, or extract from git remote)
if [[ -z "${GITLAB_PROJECT:-}" ]]; then
    if [[ -n "${CI_PROJECT_PATH:-}" ]]; then
        export GITLAB_PROJECT="$CI_PROJECT_PATH"
    else
        # Try to extract from git remote
        if git_url=$(git remote get-url origin 2>/dev/null); then
            # Extract project path from URLs like:
            # https://gitlab.com/group/subgroup/repo.git
            # git@gitlab.com:group/subgroup/repo.git
            if [[ "$git_url" =~ gitlab\.com[:/]([^/]+/.+?)(\.git)?$ ]]; then
                export GITLAB_PROJECT="${BASH_REMATCH[1]}"
            fi
        fi
    fi
fi

# Run todo-curator with all arguments passed through
exec ~/.cargo/bin/todo-curator check-mr-todos "$@"
