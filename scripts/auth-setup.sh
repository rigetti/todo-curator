#!/usr/bin/env bash

# Helper script to set up GITLAB_TOKEN and GITHUB_TOKEN for todo-curator
# This is sourced by autoauth-todo-curator.sh

set -euo pipefail

# Resolve GitLab token with explicit precedence:
# 1) GITLAB_TOKEN
# 2) GL_TOKEN
# 3) glab auth token
if [[ -n "${GITLAB_TOKEN:-}" ]]; then
    : # keep caller-provided canonical var
elif [[ -n "${GL_TOKEN:-}" ]]; then
    GITLAB_TOKEN="$GL_TOKEN"
else
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

# Resolve GitHub token with explicit precedence:
# 1) GITHUB_TOKEN
# 2) GH_TOKEN
# 3) gh auth token
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    : # keep caller-provided canonical var
elif [[ -n "${GH_TOKEN:-}" ]]; then
    GITHUB_TOKEN="$GH_TOKEN"
else
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
