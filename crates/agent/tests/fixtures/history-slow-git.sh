#!/bin/bash
set -eu
case " $* " in
  *" --git-path objects "*)
    if [ ! -e "$HISTORY_CAPTURE_READY" ]; then
      : > "$HISTORY_CAPTURE_READY"
      exec sleep 1000
    fi
    ;;
esac
exec "$HISTORY_REAL_GIT" "$@"
