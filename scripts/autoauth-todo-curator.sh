#!/usr/bin/env bash

set -euo pipefail

hash todo-curator >/dev/null

# Check and set GITLAB_TOKEN if not already set
if [[ -z "${GITLAB_TOKEN:=${GL_TOKEN:-}}" ]]; then
    if glab auth status &>/dev/null; then
        gitlab_token="$(glab config get token --host gitlab.com)"
        gitlab_host="${GITLAB_URL:-gitlab.com}"
        gitlab_host="${gitlab_host#https://}"
        gitlab_host="${gitlab_host#http://}"
        gitlab_host="${gitlab_host%/}"

        private_status="$(curl -sS -o /dev/null -w '%{http_code}' "https://${gitlab_host}/api/v4/user" -H "PRIVATE-TOKEN: ${gitlab_token}" || true)"
        bearer_status="$(curl -sS -o /dev/null -w '%{http_code}' "https://${gitlab_host}/api/v4/user" -H "Authorization: Bearer ${gitlab_token}" || true)"

        if [[ "$private_status" == "200" ]]; then
            export GITLAB_TOKEN_TYPE="pat"
        elif [[ "$bearer_status" == "200" ]]; then
            export GITLAB_TOKEN_TYPE="oauth2"
        else
            echo "The token configured in glab is not accepted by GitLab API (PRIVATE-TOKEN or Bearer)." >&2
            echo "Set GITLAB_TOKEN (or GL_TOKEN) to a GitLab PAT with 'api' scope." >&2
            exit 1
        fi

        GITLAB_TOKEN="$gitlab_token"
    else
        echo "'glab' is either not working or not authorized." >&2
        echo "See https://docs.gitlab.com/editor_extensions/gitlab_cli/#authenticate-with-gitlab" >&2
        exit 1
    fi
fi

# Check and set GITHUB_TOKEN if not already set
if [[ -z "${GITHUB_TOKEN:=${GH_TOKEN:-}}" ]]; then
    if gh auth status &>/dev/null; then
        GITHUB_TOKEN=$(gh auth token)
    else
        echo "'gh' is either not working or not authorized." >&2
        echo "See https://cli.github.com/manual/gh_auth_login" >&2
        exit 1
    fi
fi

export GITLAB_TOKEN
export GITHUB_TOKEN

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
