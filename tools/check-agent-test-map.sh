#!/usr/bin/env bash
# check-agent-test-map.sh — validates tests/agent-go-map.tsv against tests/unit/*.bats
# Checks test existence, 1-to-1 mapping, valid classifications, rounds, and statuses.
set -euo pipefail

ROOT=${CHECK_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
QUIET=0
MAP_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -q|--quiet)
      QUIET=1
      shift
      ;;
    -*)
      echo "check-agent-test-map: unknown option $1" >&2
      exit 1
      ;;
    *)
      if [[ -z $MAP_FILE ]]; then
        MAP_FILE=$1
      else
        echo "check-agent-test-map: unexpected argument $1" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

MAP_FILE=${MAP_FILE:-$ROOT/tests/agent-go-map.tsv}

# error msg... — prints an error message to stderr
err() {
  echo "check-agent-test-map: $*" >&2
}

# validate_map path — verifies existence, format, completeness and consistency of test map
validate_map() {
  local map_path=$1
  if [[ ! -f $map_path ]]; then
    err "map file not found: $map_path"
    return 1
  fi

  local line_no=0
  local errors=0
  local total_rows=0
  local blackbox_count=0
  local source_count=0
  local planned_count=0
  local active_count=0
  local done_count=0

  declare -A seen_tests=()

  while IFS=$'\t' read -r bats_file test_name classification round go_test status extra; do
    line_no=$((line_no + 1))
    if [[ $bats_file == "bats_file" ]]; then
      continue
    fi
    if [[ -n ${extra:-} ]]; then
      err "line $line_no: unexpected extra columns in $map_path"
      errors=$((errors + 1))
      continue
    fi
    if [[ -z ${bats_file:-} || -z ${test_name:-} || -z ${classification:-} || -z ${round:-} || -z ${go_test:-} || -z ${status:-} ]]; then
      err "line $line_no: missing required column(s)"
      errors=$((errors + 1))
      continue
    fi

    local key="$bats_file	$test_name"
    if [[ -n ${seen_tests["$key"]:-} ]]; then
      err "line $line_no: duplicate entry for ($bats_file, $test_name)"
      errors=$((errors + 1))
    fi
    seen_tests["$key"]=1

    case "$classification" in
      blackbox) blackbox_count=$((blackbox_count + 1)) ;;
      source)   source_count=$((source_count + 1)) ;;
      *)
        err "line $line_no: invalid classification '$classification' (expected 'blackbox' or 'source')"
        errors=$((errors + 1))
        ;;
    esac

    if [[ ! $round =~ ^R([0-9]{2})[A-Z]?$ ]]; then   # a split round has a letter after its number: R29B
      err "line $line_no: invalid round format '$round' (expected R01-R56, a split one as R29B)"
      errors=$((errors + 1))
    else
      local r_num=$((10#${BASH_REMATCH[1]}))
      if (( r_num < 1 || r_num > 56 )); then
        err "line $line_no: invalid round format '$round' (expected R01-R56, a split one as R29B)"
        errors=$((errors + 1))
      fi
    fi

    case "$status" in
      planned) planned_count=$((planned_count + 1)) ;;
      active)  active_count=$((active_count + 1)) ;;
      done)    done_count=$((done_count + 1)) ;;
      *)
        err "line $line_no: invalid status '$status' (expected planned, active, or done)"
        errors=$((errors + 1))
        ;;
    esac

    total_rows=$((total_rows + 1))
  done < "$map_path"

  if [[ ! -d "$ROOT/tests/unit" ]]; then
    err "unit tests directory not found: $ROOT/tests/unit"
    return 1
  fi

  # Verify every test in tests/unit/*.bats exists in map
  local unit_count=0
  local unit_files=0
  for f in "$ROOT"/tests/unit/*.bats; do
    [[ -f $f ]] || continue
    unit_files=$((unit_files + 1))
    local rel_file="tests/unit/${f##*/}"
    while IFS= read -r tname; do
      [[ -n $tname ]] || continue
      unit_count=$((unit_count + 1))
      local ukey="$rel_file	$tname"
      if [[ -z ${seen_tests["$ukey"]:-} ]]; then
        err "test missing from map: $rel_file :: $tname"
        errors=$((errors + 1))
      fi
    done < <(sed -n 's/^@test "\([^"]*\)".*/\1/p' "$f")
  done

  if ((unit_files == 0)); then
    err "no Bats test files found under $ROOT/tests/unit"
    return 1
  fi

  if ((errors > 0)); then
    err "validation failed with $errors error(s)"
    return 1
  fi

  if ((total_rows != unit_count)); then
    err "total map rows ($total_rows) does not match Bats tests count ($unit_count)"
    return 1
  fi

  if ((QUIET == 0)); then
    echo "check-agent-test-map: OK"
    echo "  files:          $unit_files"
    echo "  total tests:    $total_rows"
    echo "  blackbox cases: $blackbox_count"
    echo "  source cases:   $source_count"
    echo "  planned:        $planned_count"
    echo "  active:         $active_count"
    echo "  done:           $done_count"
  fi
}

validate_map "$MAP_FILE"
