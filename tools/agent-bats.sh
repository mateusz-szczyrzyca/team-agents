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
EVIDENCE_FILE=""
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
  --evidence <path>           path to write JSON execution evidence (default: <repo-root>/build/agent-bats-evidence.json)
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
    --evidence)
      EVIDENCE_FILE="$2"
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
EVIDENCE_FILE=${EVIDENCE_FILE:-$REPO_ROOT/build/agent-bats-evidence.json}

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
  printf '%s\n' "$1" | sed -e 's/[][(){}|+*^$?.\]/\\&/g'
}

# compute_map_hash — calculates sha256 of the map file
compute_map_hash() {
  if [[ -f $MAP_FILE ]]; then
    sha256sum "$MAP_FILE" | awk '{print $1}'
  else
    echo ""
  fi
}

# compute_tests_hash — calculates deterministic sha256 digest of all files in tests/unit
compute_tests_hash() {
  if [[ -d "$REPO_ROOT/tests/unit" ]]; then
    (cd "$REPO_ROOT" && find tests/unit -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | awk '{print $1}')
  else
    echo ""
  fi
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
    if [[ -n $EVIDENCE_FILE ]]; then
      mkdir -p "$(dirname "$EVIDENCE_FILE")"
      map_hash=$(compute_map_hash)
      tests_hash=$(compute_tests_hash)
      jq -n \
        --arg lane "$LANE" \
        --arg map_file "$MAP_FILE" \
        --arg map_sha256 "$map_hash" \
        --arg tests_sha256 "$tests_hash" \
        --arg candidate_bin "$CANDIDATE_BIN" \
        '{lane: $lane, map_file: $map_file, map_sha256: $map_sha256, tests_sha256: $tests_sha256, candidate_bin: $candidate_bin, selected_count: 0, executed_count: 0, failures: 0, skips: 0}' > "$EVIDENCE_FILE"
    fi
    exit 0
  fi

  STAGED_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/staged-agent-XXXXXX")
  REPORTS_DIR=$(mktemp -d "${TMPDIR:-/tmp}/agent-bats-reports-XXXXXX")
  cleanup_candidate_root() {
    local pkill_rc=0
    pkill -f -- "$STAGED_ROOT" 2>&1 || pkill_rc=$?
    # pkill returns 1 when no matching processes were found, which is normal on exit.
    if (( pkill_rc != 0 && pkill_rc != 1 )); then
      echo "agent-bats: warning: pkill exited with status $pkill_rc" >&2
    fi
    rm -rf "$STAGED_ROOT" "$REPORTS_DIR"
  }
  trap cleanup_candidate_root EXIT

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

  # Verify executed count vs selected count and gather outcomes
  report_files=()
  mapfile -t report_files < <(compgen -G "$REPORTS_DIR/*/report.xml" || (($? == 1)))
  if ((${#report_files[@]} == 0)); then
    echo "agent-bats: no JUnit XML reports generated in $REPORTS_DIR" >&2
    exit 1
  fi
  counts=$(awk '
    /<testcase / { t++ }
    /<failure /  { f++ }
    /<skipped/   { s++ }
    END { printf "%d %d %d\n", t+0, f+0, s+0 }
  ' "${report_files[@]}")
  read -r executed_count failures_count skips_count <<< "$counts"

  if [[ -n $EVIDENCE_FILE ]]; then
    mkdir -p "$(dirname "$EVIDENCE_FILE")"
    map_hash=$(compute_map_hash)
    tests_hash=$(compute_tests_hash)
    jq -n \
      --arg lane "$LANE" \
      --arg map_file "$MAP_FILE" \
      --arg map_sha256 "$map_hash" \
      --arg tests_sha256 "$tests_hash" \
      --arg candidate_bin "$CANDIDATE_BIN" \
      --argjson selected_count "$total_selected" \
      --argjson executed_count "$executed_count" \
      --argjson failures "$failures_count" \
      --argjson skips "$skips_count" \
      '{lane: $lane, map_file: $map_file, map_sha256: $map_sha256, tests_sha256: $tests_sha256, candidate_bin: $candidate_bin, selected_count: $selected_count, executed_count: $executed_count, failures: $failures, skips: $skips}' > "$EVIDENCE_FILE"
  fi

  if (( executed_count != total_selected )); then
    echo "agent-bats: executed count ($executed_count) does not match selected count ($total_selected)" >&2
    exit 1
  fi

  exit "$run_rc"

else
  # Lane A: Oracle execution
  STAGED_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/staged-oracle-XXXXXX")
  cleanup_oracle_root() {
    local pkill_rc=0
    pkill -f -- "$STAGED_ROOT" 2>&1 || pkill_rc=$?
    # pkill returns 1 when no matching processes were found, which is normal on exit.
    if (( pkill_rc != 0 && pkill_rc != 1 )); then
      echo "agent-bats: warning: pkill exited with status $pkill_rc" >&2
    fi
    rm -rf "$STAGED_ROOT"
  }
  trap cleanup_oracle_root EXIT

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
      mapfile -t all_unit < <(cd "$STAGED_ROOT" && { compgen -G "tests/unit/*.bats" || (($? == 1)); })
      bats_files="${all_unit[*]}"
    fi
  fi
  serial_files="${BATS_SERIAL:-tests/unit/node_daemon.bats tests/unit/node_two_homes.bats tests/unit/restart.bats tests/unit/review_resilience.bats tests/unit/wake_cmd.bats tests/unit/autopilot_review_cap.bats}"
  oracle_rc=0
  (cd "$STAGED_ROOT" && BATS_JOBS="$JOBS" BATS_SERIAL="$serial_files" BATS_FILES="$bats_files" "$STAGED_ROOT/tools/unit-bats.sh") || oracle_rc=$?

  if [[ -n $EVIDENCE_FILE ]]; then
    mkdir -p "$(dirname "$EVIDENCE_FILE")"
    map_hash=$(compute_map_hash)
    tests_hash=$(compute_tests_hash)
    jq -n \
      --arg lane "$LANE" \
      --arg map_file "$MAP_FILE" \
      --arg map_sha256 "$map_hash" \
      --arg tests_sha256 "$tests_hash" \
      --arg candidate_bin "" \
      --argjson failures "$oracle_rc" \
      '{lane: $lane, map_file: $map_file, map_sha256: $map_sha256, tests_sha256: $tests_sha256, candidate_bin: $candidate_bin, selected_count: 0, executed_count: 0, failures: $failures, skips: 0}' > "$EVIDENCE_FILE"
  fi

  exit "$oracle_rc"
fi
