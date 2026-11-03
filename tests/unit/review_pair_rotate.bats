#!/usr/bin/env bats
# AGENT_REVIEW_DRAW=rotate: an --autopilot review pair rotates through every investigator. A new pair is never one of
# the last two pairs (a PR's re-review after a fix pass included), prefers investigators outside the last pair and the
# pairs used least recently; it waits a while for a busy investigator rather than repeat a pair, and with no other
# pair possible it takes the best one free.

load helpers

# pair_of pr — the pair start_review_pair starts in rotate mode for a coordinator task of that PR, among the free
# investigators in FREE (four by default) out of those in ALL (FREE by default); prints "A B", or fails with no pair
pair_of() {
  local id="T$1"
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nREPO_NAME=repo\nPR_NUMBER=%s\nPR_URL=https://github.com/o/r/pull/%s\nREVIEW_PAIR_SINCE=%s\n' \
    "$id" "$1" "$1" "${SINCE:-$(date +%s)}" > "$AGENTS_HOME/tasks/$id/meta.env"
  ( source "$REPO_ROOT/bin/agent"
    AGENT_REVIEW_DRAW=rotate
    # shellcheck disable=SC2086  # FREE and ALL are lists of names
    free_investigator_list() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    # shellcheck disable=SC2086
    investigator_names() { printf '%s\n' ${ALL:-${FREE:-inv-a inv-b inv-c inv-d}}; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; }
    act() { :; }
    auto_stage() { :; }
    start_review_pair "$id" > /dev/null || exit 1
    d=$AGENTS_HOME/reviews/repo-$1
    echo "$(cat "$d/A.agent") $(cat "$d/B.agent")" )
}

@test "rotate: A,B then C,D; every new pair differs from the last two; four investigators make all six pairs" {
  local pairs=() p i
  for i in 1 2 3 4 5 6; do p=$(pair_of "$i"); pairs+=("$p"); done
  [ "${pairs[0]}" = "inv-a inv-b" ]
  [ "${pairs[1]}" = "inv-c inv-d" ]
  for i in 2 3 4 5; do
    [ "${pairs[i]}" != "${pairs[i-1]}" ]
    [ "${pairs[i]}" != "${pairs[i-2]}" ]
  done
  [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]
}

@test "rotate: a PR's re-review after a fix pass is done by another pair, not the one before" {
  first=$(pair_of 9)
  second=$(pair_of 9)
  third=$(pair_of 9)
  [ "$second" != "$first" ]
  [ "$third" != "$first" ] && [ "$third" != "$second" ]
}

@test "rotate: the history is shared by every PR and repository" {
  [ "$(pair_of 1)" = "inv-a inv-b" ]
  [ "$(pair_of 2)" = "inv-c inv-d" ]
  [ "$(wc -l < "$AGENTS_HOME/reviews/history")" -eq 2 ]
}

@test "rotate: with two investigators their pair reviews again — the autopilot does not stall" {
  [ "$(FREE="inv-a inv-b" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b" pair_of 2)" = "inv-a inv-b" ]
}

@test "rotate: the only new pair has a busy investigator — it waits AGENT_REVIEW_ROTATE_WAIT_MIN, then repeats a pair" {
  [ "$(FREE="inv-a inv-b inv-c" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b inv-c" pair_of 2)" = "inv-a inv-c" ]
  # only inv-b inv-c is new; inv-c is busy
  FREE="inv-a inv-b" ALL="inv-a inv-b inv-c" run pair_of 3
  [ "$status" -ne 0 ]
  [ "$(wc -l < "$AGENTS_HOME/reviews/history")" -eq 2 ]
  # waited longer than AGENT_REVIEW_ROTATE_WAIT_MIN (20): the best free pair, the least recent one
  [ "$(SINCE=$(($(date +%s) - 1300)) FREE="inv-a inv-b" ALL="inv-a inv-b inv-c" pair_of 3)" = "inv-a inv-b" ]
}

@test "rotate: three investigators — the third pair is the one not used yet" {
  [ "$(FREE="inv-a inv-b inv-c" pair_of 1)" = "inv-a inv-b" ]
  [ "$(FREE="inv-a inv-b inv-c" pair_of 2)" = "inv-a inv-c" ]
  [ "$(FREE="inv-a inv-b inv-c" pair_of 3)" = "inv-b inv-c" ]
}

@test "rotate: a re-review by another investigator gets the slot's previous review as someone else's" {
  local dir="$AGENTS_HOME/reviews/repo-5" q
  mkdir -p "$dir" "$AGENTS_HOME/tasks/T5"
  printf 'ID=T5\nREPO_NAME=repo\nPR_NUMBER=5\nPR_URL=https://github.com/o/r/pull/5\nFIX_PASS=1\n' > "$AGENTS_HOME/tasks/T5/meta.env"
  echo inv-a > "$dir/A.agent"
  echo "1. major: the cache never expires" > "$dir/A.md"
  q=$( source "$REPO_ROOT/bin/agent"
       bus() { if [[ $1 == inv-add ]]; then while (($#)); do [[ $1 != --question ]] || printf '%s' "$2" > "$AGENTS_HOME/q"; shift; done; echo 3; fi; }
       start_investigation() { :; }
       remember_investigator() { :; }
       start_review_slot T5 "$dir" A inv-c > /dev/null
       cat "$AGENTS_HOME/q" )
  [[ $q == *"the cache never expires"* ]]
  [[ $q == *"by inv-a"* ]]
  [[ $q != *"Your previous review"* ]]
}

@test "rotate: the supervise timer dispatches a waiting review pair, which no run's end may do" {
  mkdir -p "$AGENTS_HOME/tasks/T1"
  printf 'ID=T1\nSTATUS=done\nREVIEW_PAIR_PENDING=1\nREVIEW_PAIR_SINCE=%s\n' "$(date +%s)" > "$AGENTS_HOME/tasks/T1/meta.env"
  out=$( source "$REPO_ROOT/bin/agent"; dispatch() { echo dispatched; }; AGENT_REVIEW_DRAW=rotate; cmd__supervise )
  [[ $out == *dispatched* ]]
  out=$( source "$REPO_ROOT/bin/agent"; dispatch() { echo dispatched; }; cmd__supervise )
  [[ $out != *dispatched* ]]   # random and list: as before, a run's end dispatches
}
