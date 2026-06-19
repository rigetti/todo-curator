#!/usr/bin/env bash

set -euo pipefail

hash todo-curator >/dev/null

# Source auth setup helper
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./auth-setup.sh
source "${script_dir}/auth-setup.sh"

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
            gitlab_project=""
            case "$git_url" in
                *gitlab.com:*)
                    gitlab_project="${git_url#*gitlab.com:}"
                    ;;
                *gitlab.com/*)
                    gitlab_project="${git_url#*gitlab.com/}"
                    ;;
            esac

            if [[ -n "$gitlab_project" ]]; then
                export GITLAB_PROJECT="${gitlab_project%.git}"
            fi
        fi
    fi
fi

# Run todo-curator with all arguments passed through
exec todo-curator "$@"
