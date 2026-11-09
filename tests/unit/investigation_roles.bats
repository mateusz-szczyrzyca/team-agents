#!/usr/bin/env bats
# --investigation drew its merger and its reviewer from the team's own investigators only. The merger is to be an
# investigator outside the team, drawn at random among the free ones; the reviewer another one, never the merger,
# outside the team while one is left, else from the team. Only when no investigator is outside the team does the
# merger come from it (one that finished phase 1). With investigators a, b, c and d:
#   team a     — merger one of b, c, d; reviewer one of the other two (never a, never the merger)
#   team a,b   — merger c or d; reviewer the other of the two
#   team a,b,c — merger d; reviewer one of a, b, c
# An outsider that is busy is waited for, not replaced by a team member.

load helpers

# four — investigators a, b, c, d in agents.conf. bin/agent is then loaded in the test itself: loaded inside a
# function, agents.conf's declare -A would make its arrays local to that function
four() {
  sed -i 's/^declare -A INVESTIGATORS_TYPE=.*/declare -A INVESTIGATORS_TYPE=([a]=claude [b]=claude [c]=claude [d]=claude)/' \
    "$AGENTS_HOME/agents.conf"
  grep -q '^declare -A INVESTIGATORS_TYPE=(\[a\]' "$AGENTS_HOME/agents.conf"
  unset AGENT_INVESTIGATION_SEED
}

# parent id team [merger] — an investigation team's coordinator task: its team (all done in phase 1), its merger
parent() {
  local m members=""
  mkdir -p "$TASKS/$1"
  printf 'ID=%s\nKIND=investigation_team\nINVESTIGATION_PIPELINE=1\nINVESTIGATION_TEAM=%s\nINVESTIGATION_MERGER=%s\n' \
    "$1" "$2" "${3:-}" > "$TASKS/$1/meta.env"
  for m in ${2//,/ }; do members+="${members:+,}{\"agent\": \"$m\", \"status\": \"done\"}"; done
  printf '{"members": [%s]}\n' "$members" > "$TASKS/$1/phase1.json"
}

# drawn parent phase — every investigator the phase's draw gave in 40 draws, sorted, space-separated
drawn() {
  local i
  for ((i = 0; i < 40; i++)); do
    investigation_phase_pick "$1" "$2"
    echo "$INVESTIGATION_PICK"
  done | sort -u | paste -sd' ' -
}

@test "a team of one: its merger is one of the three others, its reviewer one of the two left" {
  four
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  parent P a
  [ "$(drawn P 2)" = "b c d" ]
  parent P a b
  [ "$(drawn P 3)" = "c d" ]
}

@test "a team of two: its merger is one of the two others, its reviewer the last one" {
  four
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  parent P a,b
  [ "$(drawn P 2)" = "c d" ]
  parent P a,b c
  [ "$(drawn P 3)" = "d" ]
}

@test "a team of three: its merger is the last investigator, its reviewer one of the team" {
  four
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  parent P a,b,c
  [ "$(drawn P 2)" = "d" ]
  parent P a,b,c d
  [ "$(drawn P 3)" = "a b c" ]
}

@test "a team of every investigator: its merger comes from the team, its reviewer never the merger" {
  four
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  parent P a,b,c,d
  [ "$(drawn P 2)" = "a b c d" ]
  parent P a,b,c,d b
  [ "$(drawn P 3)" = "a c d" ]
}

@test "an outsider that is busy is waited for: no team member takes its place" {
  four
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  parent P a,b
  mkdir -p "$TASKS/C1" "$TASKS/D1"
  printf 'ID=C1\nAGENT=c\nKIND=investigation\nSTATUS=running\n' > "$TASKS/C1/meta.env"
  printf 'ID=D1\nAGENT=d\nKIND=investigation\nSTATUS=running\n' > "$TASKS/D1/meta.env"
  investigation_phase_pick P 2
  [ -z "$INVESTIGATION_PICK" ]
  [ "$INVESTIGATION_ELIGIBLE" -eq 2 ]
}
