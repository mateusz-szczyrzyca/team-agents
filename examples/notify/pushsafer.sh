#!/usr/bin/env bash
# Notification hook for Pushsafer (https://www.pushsafer.com).
# agents.conf:  NOTIFY_CMD="$HOME/agents/notify/pushsafer.sh"
# Variables (put them in ~/agents/secrets.env — notifications are sent from agent runs, which load it):
#   PUSHSAFER_KEY (private or alias key), optional PUSHSAFER_DEVICE (default: all devices "a").
set -euo pipefail
f=$1
title=$(head -n1 "$f")
body=$(tail -n +2 "$f" | head -c 4000)
curl -fsS https://www.pushsafer.com/api \
  --data-urlencode "k=${PUSHSAFER_KEY:?set PUSHSAFER_KEY}" \
  --data-urlencode "d=${PUSHSAFER_DEVICE:-a}" \
  --data-urlencode "t=agents: $title" \
  --data-urlencode "m=$body" >/dev/null
