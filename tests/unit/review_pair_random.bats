#!/usr/bin/env bats
# A review pair is drawn at random among the free investigators — two different ones —, not always the first two of
# the list; A and B keep the list's order. A PR's next review pass keeps its pair: each re-checks its own findings.

load helpers

# pair_of pr seed — the pair start_review_pair starts for a coordinator task of that PR, among the free investigators
# in FREE (four by default), with the draw fixed by the seed; prints "A B"
pair_of() {
  local id="T$1"
  mkdir -p "$AGENTS_HOME/tasks/$id"
  printf 'ID=%s\nREPO_NAME=repo\nPR_NUMBER=%s\nPR_URL=https://github.com/o/r/pull/%s\n' "$id" "$1" "$1" \
    > "$AGENTS_HOME/tasks/$id/meta.env"
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2086  # FREE is a list of names
    free_investigator_list() { printf '%s\n' ${FREE:-inv-a inv-b inv-c inv-d}; }
    start_review_slot() { echo "$4" > "$2/$3.agent"; echo 1 > "$2/$3.inv"; }
    act() { :; }
    auto_stage() { :; }
    AGENT_REVIEW_SEED=$2 start_review_pair "$id" > /dev/null
    d=$AGENTS_HOME/reviews/repo-$1
    echo "$(cat "$d/A.agent") $(cat "$d/B.agent")" )
}

@test "a review pair is drawn at random: over eight PRs each of four free investigators reviews, never twice in a pair" {
  seen=""
  for n in 1 2 3 4 5 6 7 8; do
    read -r a b <<< "$(pair_of "$n" "$n")"
    [ -n "$a" ] && [ -n "$b" ] && [ "$a" != "$b" ]
    seen+=" $a $b"
  done
  for i in inv-a inv-b inv-c inv-d; do [[ $seen == *" $i"* ]]; done
}

@test "with two free investigators the pair keeps the list's order: A the first, B the second" {
  for s in 1 2 3 4; do [ "$(FREE="inv-a inv-b" pair_of "1$s" "$s")" = "inv-a inv-b" ]; done
}

@test "a PR's next review pass keeps its pair" {
  first=$(pair_of 9 1)
  [ "$(pair_of 9 7)" = "$first" ]
}

@test "AGENT_REVIEW_DRAW=list: the pair is the first two free investigators of the list, as before the draw" {
  for s in 1 2 3 4 5; do [ "$(AGENT_REVIEW_DRAW=list pair_of "2$s" "$s")" = "inv-a inv-b" ]; done
}
