#!/usr/bin/env bash
set -euo pipefail

# Replay the recorded capacity refusal without calling a provider.
printf '%s\n' "$@" > .fake-args
case $(cat .fake-scenario) in
  at-capacity)
    cat "$(dirname -- "$0")/../cli/codex/at-capacity.jsonl"
    exit 1
    ;;
  *)
    printf '%s\n' 'unknown fake Codex scenario' >&2
    exit 2
    ;;
esac
