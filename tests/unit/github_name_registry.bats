#!/usr/bin/env bats
# An agent's name keeps its GitHub colour in every round, whatever the configuration's order of the agents is later:
# the first time a run can publish a name the runner gives it a colour of AGENT_GITHUB_COLORS that no name has yet
# and remembers it in $AGENTS_HOME/github-colors ("name #RRGGBB" a line). Investigators and team agents never share
# a colour while the palette has a free one.

load helpers

# names_env [conf...] — AGENT_GITHUB_NAME_COLORS as agents_gh_env sets it for a run of AGENT (TEAM: its team's task
# files) after appending those lines to agents.conf
names_env() {
  local l
  for l in "$@"; do printf '%s\n' "$l" >> "$AGENTS_HOME/agents.conf"; done
  ( source "$REPO_ROOT/bin/agent"
    RUNNER_ID="" REPO_ROOT=$HOME/repo AGENT_GITHUB_TRUSTED=o agents_gh_env
    printf '%s\n' "$AGENT_GITHUB_NAME_COLORS" )
}

# colour_of name map — that name's colour in a "name=#c …" map
colour_of() {
  local e
  for e in $2; do [[ ${e%%=*} != "$1" ]] || { printf '%s\n' "${e#*=}"; return 0; }; done
  return 1
}

# team_task id agent team — a task file of that agent in that team (what attribution_block reads)
team_task() {
  mkdir -p "$AGENTS_HOME/tasks/$1"
  printf 'ID=%s\nAGENT=%s\nTEAM=%s\n' "$1" "$2" "$3" > "$AGENTS_HOME/tasks/$1/meta.env"
}

setup_team() {
  team_task T1-gemini-1 gemini-1 T1
  team_task T1-claude-haiku claude-haiku T1
  team_task T1-codex-1 codex-1 T1
  cat >> "$AGENTS_HOME/agents.conf" <<'CONF'
declare -A AGENT_TYPE=([gemini-1]=gemini [claude-haiku]=claude [codex-1]=codex [claude-sonnet]=claude)
declare -A INVESTIGATORS_TYPE=([inv-opus]=claude [inv-astra]=codex [inv-kimi]=opencode [inv-gemini-argon]=gemini)
AGENT_GITHUB_COLORS="#000001 #000002 #000003 #000004 #000005 #000006 #000007 #000008"
CONF
}

@test "a team's names keep their colours when NORMAL_AGENTS changes between rounds" {
  setup_team
  first=$(AGENT=gemini-1 TEAM=T1 names_env 'NORMAL_AGENTS=(claude-haiku codex-1 gemini-1)')
  second=$(AGENT=gemini-1 TEAM=T1 names_env 'NORMAL_AGENTS=(codex-1 gemini-1 claude-sonnet)')
  for n in gemini-1 claude-haiku codex-1 inv-astra inv-opus; do
    [ -n "$(colour_of "$n" "$first")" ]
    [ "$(colour_of "$n" "$second")" = "$(colour_of "$n" "$first")" ]
  done
}

@test "the coordinator, the members and every investigator each get a colour nobody else has" {
  setup_team
  map=$(AGENT=gemini-1 TEAM=T1 names_env 'NORMAL_AGENTS=(codex-1 gemini-1)')
  cols=()
  for n in gemini-1 claude-haiku codex-1 inv-opus inv-astra inv-kimi inv-gemini-argon; do cols+=("$(colour_of "$n" "$map")"); done
  [ "$(printf '%s\n' "${cols[@]}" | grep -c '^#')" -eq 7 ]
  [ "$(printf '%s\n' "${cols[@]}" | sort -u | wc -l)" -eq 7 ]
}

@test "the colours are remembered in github-colors, one name a line; a later team adds its new names only" {
  setup_team
  AGENT=gemini-1 TEAM=T1 names_env > /dev/null
  f=$AGENTS_HOME/github-colors
  [ "$(grep -c . "$f")" -eq 7 ]
  grep -qE '^gemini-1 #[0-9A-Fa-f]{6}$' "$f"
  before=$(grep '^codex-1 ' "$f")
  team_task T2-claude-sonnet claude-sonnet T2
  team_task T2-codex-1 codex-1 T2
  map=$(AGENT=claude-sonnet TEAM=T2 names_env)
  [ "$(grep -c . "$f")" -eq 8 ]
  [ "$(grep '^codex-1 ' "$f")" = "$before" ]
  [ "$(colour_of claude-sonnet "$map")" = "#000008" ]
}

@test "a palette used up: a new name gets the colour the fewest names have" {
  setup_team
  printf '%s\n' 'inv-opus #000001' 'inv-astra #000002' 'inv-kimi #000003' 'inv-gemini-argon #000004' 'a #000005' \
    'b #000006' 'c #000007' 'd #000008' 'e #000001' > "$AGENTS_HOME/github-colors"
  map=$(AGENT=claude-sonnet names_env)
  [ "$(colour_of claude-sonnet "$map")" = "#000002" ]
}

@test "two runs starting at once never give two names one colour" {
  setup_team
  for n in gemini-1 claude-haiku codex-1 claude-sonnet; do
    ( AGENT=$n names_env > /dev/null ) &
  done
  wait
  [ "$(cut -d' ' -f2 "$AGENTS_HOME/github-colors" | sort | uniq -d | wc -l)" -eq 0 ]
  [ "$(grep -c . "$AGENTS_HOME/github-colors")" -eq 8 ]
}

@test "AGENT_GITHUB_COLORS=none: no colours, nothing remembered" {
  setup_team
  map=$(AGENT=gemini-1 TEAM=T1 names_env 'AGENT_GITHUB_COLORS=none')
  [ -z "$map" ]
  [ ! -s "$AGENTS_HOME/github-colors" ]
}
