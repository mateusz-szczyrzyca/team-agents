#!/usr/bin/env bats
# A review pair prefers investigators whose model the last review pair did not use: after the fewest members of the
# last pair, the fewest of its models (CLI and model, INVESTIGATORS_TYPE / INVESTIGATORS_MODEL), in every draw mode.

load helpers

# pick draw seed — the pair review_pair_select takes among six investigators after the review history "a b": a and c
# run codex/astra, b and d claude/opus, e gemini/flash, f codex/sol
pick() {
  ( source "$REPO_ROOT/bin/agent"
    # shellcheck disable=SC2034  # read by the sourced script's review_pair_select
    declare -A INVESTIGATORS_TYPE=([a]=codex [b]=claude [c]=codex [d]=claude [e]=gemini [f]=codex)
    # shellcheck disable=SC2034  # as above
    declare -A INVESTIGATORS_MODEL=([a]=astra [b]=opus [c]=astra [d]=opus [e]=flash [f]=sol)
    mkdir -p "$AGENTS_HOME/reviews"
    echo 'a b' > "$AGENTS_HOME/reviews/history"
    AGENT_REVIEW_DRAW=$1 AGENT_REVIEW_SEED=$2 review_pair_select strict "" a b c d e f )
}

@test "rotate: of the pairs without a member of the last pair, the one without its models" {
  [ "$(pick rotate 1)" = 'e f' ]
}

@test "random: of the pairs without a member of the last pair, the one without its models, whatever the draw" {
  local s
  for s in 1 2 3 4 5 6 7 8; do [ "$(pick random "$s")" = 'e f' ]; done
}

@test "investigators of one model: the model does not narrow the draw" {
  local s seen=()
  for s in 1 2 3 4 5 6 7 8 9 10 11 12; do
    seen+=("$( source "$REPO_ROOT/bin/agent"
      # shellcheck disable=SC2034  # read by the sourced script's review_pair_select
      declare -A INVESTIGATORS_TYPE=([a]=claude [b]=claude [c]=claude [d]=claude [e]=claude)
      # shellcheck disable=SC2034  # as above
      declare -A INVESTIGATORS_MODEL=()
      mkdir -p "$AGENTS_HOME/reviews"
      echo 'a b' > "$AGENTS_HOME/reviews/history"
      AGENT_REVIEW_DRAW=random AGENT_REVIEW_SEED=$s review_pair_select strict "" a b c d e )")
  done
  [ "$(printf '%s\n' "${seen[@]}" | sort -u | wc -l)" -gt 1 ]
}
