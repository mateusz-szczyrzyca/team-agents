#!/usr/bin/env bash
# agent-bats.sh — runs Bats tests in a staged private repository root for Lane A (oracle)
# or Lane B (Go candidate binary).
#
# In Lane B, only ledger rows in tests/agent-go-map.tsv classified as "blackbox" and in
# state "active" or "done" are executed against build/agent-go. Mixed files use Bats --filter
# with an anchored, regex-escaped allowlist of exact test names.
#
# In Lane A, the unchanged legacy suite runs against the Bash oracle.
#
# Selected and executed case counts are verified against JUnit reports.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
LANE="candidate"
CANDIDATE_BIN=""
MAP_FILE=""
JOBS=${BATS_JOBS:-1}

# show_usage — prints usage information
show_usage() {
  cat <<'USAGE'
usage: agent-bats.sh [options] [bats_files...]

options:
  --lane <candidate|oracle>   select Lane B candidate (default) or Lane A oracle
  --repo-root <dir>           repository root directory (default: repo containing this script)
  --candidate-bin <path>      path to Go candidate binary (default: <repo-root>/build/agent-go)
  --map-file <path>           path to agent-go-map.tsv (default: <repo-root>/tests/agent-go-map.tsv)
  --jobs <N>                  number of parallel jobs (default: BATS_JOBS or 1)
  -h, --help                  show this help message
USAGE
}

FILES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --lane)
      LANE="$2"
      shift 2
      ;;
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    --candidate-bin)
      CANDIDATE_BIN="$2"
      shift 2
      ;;
    --map-file)
      MAP_FILE="$2"
      shift 2
      ;;
    --jobs)
      JOBS="$2"
      shift 2
      ;;
    -h|--help)
      show_usage
      exit 0
      ;;
    -*)
      echo "agent-bats: unknown option $1" >&2
      exit 1
      ;;
    *)
      FILES+=("$1")
      shift
      ;;
  esac
done

CANDIDATE_BIN=${CANDIDATE_BIN:-$REPO_ROOT/build/agent-go}
MAP_FILE=${MAP_FILE:-$REPO_ROOT/tests/agent-go-map.tsv}

if [[ $LANE != "candidate" && $LANE != "oracle" ]]; then
  echo "agent-bats: invalid lane '$LANE' (must be 'candidate' or 'oracle')" >&2
  exit 1
fi

if [[ $LANE == "candidate" && ! -f $CANDIDATE_BIN ]]; then
  echo "agent-bats: candidate binary not found: $CANDIDATE_BIN" >&2
  exit 1
fi

# stage_repository dest_dir — copies necessary repository files into an isolated staged root
stage_repository() {
  local dest=$1
  mkdir -p "$dest"
  # Copy working files, skipping git, build and transient directories
  for item in "$REPO_ROOT"/*; do
    local base
    base=$(basename "$item")
    case "$base" in
      build|.git|.agents|.gemini|*.tmp)
        continue
        ;;
      *)
        if [[ -e $item ]]; then
          cp -a "$item" "$dest/"
        fi
        ;;
    esac
  done
}

# escape_regex str — escapes regular expression metacharacters for Bats --filter
escape_regex() {
  printf '%s\n' "$1" | sed -e 's/[][()+*^$?.\]/\\&/g'
}

# count_junit_cases dir — counts total <testcase elements across report.xml files
count_junit_cases() {
  local report_dir=$1
  local count=0
  if compgen -G "$report_dir/*/report.xml" > /dev/null || [[ -f "$report_dir/report.xml" ]]; then
    count=$(grep -c '<testcase ' "$report_dir"/*/report.xml "$report_dir"/report.xml 2>/dev/null \
      | awk -F: '{s+=$NF} END {print s+0}')
  fi
  printf '%d\n' "$count"
}

if [[ $LANE == "candidate" ]]; then
  if [[ ! -f $MAP_FILE ]]; then
    echo "agent-bats: map file not found: $MAP_FILE" >&2
    exit 1
  fi

  # Discover active/done blackbox tests from map file
  declare -A file_tests=()
  total_selected=0

  while IFS=$'\t' read -r bats_file test_name classification _ _ state _; do
    if [[ $bats_file == "bats_file" || -z ${bats_file:-} ]]; then
      continue
    fi
    if [[ $classification == "blackbox" && ( $state == "active" || $state == "done" ) ]]; then
      total_selected=$((total_selected + 1))
      if [[ -z ${file_tests["$bats_file"]:-} ]]; then
        file_tests["$bats_file"]="$test_name"
      else
        file_tests["$bats_file"]="${file_tests["$bats_file"]}"$'\n'"$test_name"
      fi
    fi
  done < "$MAP_FILE"

  if (( total_selected == 0 )); then
    echo "agent-bats: 0 active/done blackbox tests in $MAP_FILE (nothing to run)"
    exit 0
  fi

  STAGED_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/staged-agent-XXXXXX")
  REPORTS_DIR=$(mktemp -d "${TMPDIR:-/tmp}/agent-bats-reports-XXXXXX")
  trap 'pkill -f -- "$STAGED_ROOT" 2>/dev/null || true; rm -rf "$STAGED_ROOT" "$REPORTS_DIR"' EXIT

  stage_repository "$STAGED_ROOT"

  # Place Go candidate binary at bin/agent (actual executable copy, not symlink)
  mkdir -p "$STAGED_ROOT/bin"
  rm -f "$STAGED_ROOT/bin/agent"
  cp -p "$CANDIDATE_BIN" "$STAGED_ROOT/bin/agent"
  chmod 755 "$STAGED_ROOT/bin/agent"
  if [[ -L "$STAGED_ROOT/bin/agent" ]]; then
    echo "agent-bats: bin/agent must be a regular executable copy, not a symlink" >&2
    exit 1
  fi

  # Run selected tests in staged root
  run_rc=0
  run_idx=0
  for rel_file in "${!file_tests[@]}"; do
    run_idx=$((run_idx + 1))
    target_file="$STAGED_ROOT/$rel_file"
    if [[ ! -f $target_file ]]; then
      echo "agent-bats: test file $rel_file not found in staged tree" >&2
      run_rc=1
      continue
    fi

    # Read selected test names for this file
    mapfile -t selected_names <<< "${file_tests["$rel_file"]}"

    # Build regex filter
    regex_parts=()
    for tname in "${selected_names[@]}"; do
      [[ -n $tname ]] || continue
      escaped=$(escape_regex "$tname")
      regex_parts+=("$escaped")
    done

    filter_expr=""
    if (( ${#regex_parts[@]} > 0 )); then
      filter_expr="^("
      for i in "${!regex_parts[@]}"; do
        if (( i > 0 )); then
          filter_expr+="|"
        fi
        filter_expr+="${regex_parts[i]}"
      done
      filter_expr+=")$"
    fi

    out_dir="$REPORTS_DIR/run_$run_idx"
    mkdir -p "$out_dir"
    
    # Run bats in staged root context
    bats_cmd=(bats --report-formatter junit --output "$out_dir")
    if [[ -n $filter_expr ]]; then
      bats_cmd+=(--filter "$filter_expr")
    fi
    bats_cmd+=("$target_file")

    if ! (cd "$STAGED_ROOT" && "${bats_cmd[@]}"); then
      run_rc=1
    fi
  done

  # Verify executed count vs selected count
  executed_count=$(count_junit_cases "$REPORTS_DIR")
  if (( executed_count != total_selected )); then
    echo "agent-bats: executed count ($executed_count) does not match selected count ($total_selected)" >&2
    exit 1
  fi

  exit "$run_rc"

else
  # Lane A: Oracle execution
  STAGED_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/staged-oracle-XXXXXX")
  trap 'pkill -f -- "$STAGED_ROOT" 2>/dev/null || true; rm -rf "$STAGED_ROOT"' EXIT

  stage_repository "$STAGED_ROOT"
  mkdir -p "$STAGED_ROOT/bin"

  # Place oracle at bin/agent (tracked bin/agent or tests/oracle/agent)
  rm -f "$STAGED_ROOT/bin/agent"
  if [[ -f "$REPO_ROOT/tests/oracle/agent" ]]; then
    cp -p "$REPO_ROOT/tests/oracle/agent" "$STAGED_ROOT/bin/agent"
  elif [[ -f "$REPO_ROOT/bin/agent" ]]; then
    cp -p "$REPO_ROOT/bin/agent" "$STAGED_ROOT/bin/agent"
  else
    echo "agent-bats: oracle binary/script not found" >&2
    exit 1
  fi
  chmod 755 "$STAGED_ROOT/bin/agent"
  if [[ -L "$STAGED_ROOT/bin/agent" ]]; then
    echo "agent-bats: oracle must not be a symlink" >&2
    exit 1
  fi

  # Run standard bats suite in staged root
  bats_files="${BATS_FILES:-}"
  if [[ -z $bats_files ]]; then
    if (( ${#FILES[@]} > 0 )); then
      bats_files="${FILES[*]}"
    else
      mapfile -t all_unit < <(compgen -G "$STAGED_ROOT/tests/unit/*.bats" || true)
      bats_files="${all_unit[*]}"
    fi
  fi
  (cd "$STAGED_ROOT" && BATS_JOBS="$JOBS" BATS_FILES="$bats_files" "$STAGED_ROOT/tools/unit-bats.sh")
  exit $?
fi
