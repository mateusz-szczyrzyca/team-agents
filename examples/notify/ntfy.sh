#!/usr/bin/env bash
# Notification hook for ntfy (https://ntfy.sh or self-hosted).
# agents.toml:  [notifications] command = ["/home/you/notify/ntfy.sh"]   (docs/configuration.md)
# Variables (put them in [notification_env] of secrets.toml):
#   NTFY_URL   topic URL, e.g. https://ntfy.example.com/agents
#   NTFY_TOKEN optional access token
set -euo pipefail
f=$1
title=$(head -n1 "$f")
body=$(tail -n +2 "$f")
NTFY_URL=${NTFY_URL:?set NTFY_URL}
args=(-fsS -H "Title: agents: $title" --data-binary "$body")
[[ -n ${NTFY_TOKEN:-} ]] && args+=(-H "Authorization: Bearer $NTFY_TOKEN")
curl "${args[@]}" "$NTFY_URL" >/dev/null
