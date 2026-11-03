#!/usr/bin/env bats
# Four investigators A, B, C and D review a PR pass after pass: A+B, then C+D, then the mixed pairs (A+C, B+D, A+D,
# B+C) — every one of the six pairs once before any pair reviews again, in the default draw (random) as in rotate. A
# pass never repeats the pair of the pass before while another pair is free.

load helpers

# review_pass pr [seed] — one review pass of that PR: the pair start_review_pair starts for its coordinator task among
# the free investigators in FREE (four by default), the draw fixed by the seed; then both reviews end (their verdicts
# written, as review_pair_done does). Prints "A B", or fails with no pair.
review_pass() {
  local id="T$1"
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nREPO_NAME=repo\nPR_NUMBER=%s\nPR_URL=https://github.com/o/r/pull/%s\nREVIEW_PAIR_SINCE=%s\n' \
    "$id" "$1" "$1" "$(date +%s)" > "$AGENTS_HOME/tasks/$id/meta.env"
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2086  # FREE is a list of names
    free_investigator_list() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    # shellcheck disable=SC2086  # as above
    investigator_names() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; rm -f "$2/$3.verdict"; }
    act() { :; }
    auto_stage() { :; }
    AGENT_REVIEW_SEED=${2:-1} start_review_pair "$id" > /dev/null || exit 1
    d=$AGENTS_HOME/reviews/repo-$1
    echo needs-fixes > "$d/A.verdict"; echo needs-fixes > "$d/B.verdict"
    review_pair_key "$(cat "$d/A.agent")" "$(cat "$d/B.agent")" )
}

# six_passes pr seed — six review passes of one PR, one pair a line
six_passes() {
  local k
  for k in 1 2 3 4 5 6; do review_pass "$1" "$(($2 * 7 + k))" || return 1; done
}

# disjoint pair pair — 0 when the two pairs ("a b") have no investigator in common
disjoint() {
  local x
  for x in $1; do [[ " $2 " != *" $x "* ]] || return 1; done
}

@test "default draw: one PR's six passes take all six pairs of A, B, C, D, each once — A+B, C+D, then the mixed ones" {
  local s pairs
  for s in 1 2 3 4 5 6 7 8 9 10; do
    rm -rf "$AGENTS_HOME/reviews"
    mapfile -t pairs < <(six_passes "1$s" "$s")
    [ "${#pairs[@]}" -eq 6 ]
    [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]
    disjoint "${pairs[0]}" "${pairs[1]}"
    for k in 2 3 4 5; do ! disjoint "${pairs[k]}" "${pairs[0]}" && ! disjoint "${pairs[k]}" "${pairs[1]}"; done
  done
}

@test "default draw: six PRs in a row take all six pairs, each once" {
  local s n pairs
  for s in 1 2 3 4 5 6 7 8; do
    rm -rf "$AGENTS_HOME/reviews"
    pairs=()
    for n in 1 2 3 4 5 6; do pairs+=("$(review_pass "2$s$n" "$((s * 5 + n))")"); done
    [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]
    disjoint "${pairs[0]}" "${pairs[1]}"
  done
}

@test "rotate: one PR's six passes take all six pairs, each once" {
  export AGENT_REVIEW_DRAW=rotate
  local pairs
  mapfile -t pairs < <(six_passes 31 1)
  [ "$(printf '%s\n' "${pairs[@]}" | sort -u | wc -l)" -eq 6 ]
  [ "${pairs[0]}" = "inv-a inv-b" ] && [ "${pairs[1]}" = "inv-c inv-d" ]
}

@test "default draw: a PR's pass never repeats the pair of its pass before, whichever three are free" {
  local s free prev next
  for s in 1 2 3 4 5 6 7 8; do
    rm -rf "$AGENTS_HOME/reviews"
    prev=$(review_pass "4$s" "$s")
    free=$(printf '%s\n' inv-a inv-b inv-c inv-d | grep -vx "$(printf '%s\n' inv-a inv-b inv-c inv-d | grep -vxF -e "${prev% *}" -e "${prev#* }" | head -n 1)" | paste -sd' ')
    next=$(FREE="$free" review_pass "4$s" "$((s + 1))")
    [ "$next" != "$prev" ]
  done
}
