#!/usr/bin/env bats
# Every role and continuation receives the optional Serena guidance; the PR rule's roles are in pr_rule.bats.
load helpers

# prompt_fixture prepares a real prompt renderer without launching an agent.
prompt_fixture() {
  mkdir -p "$TASKS/P"
  printf 'ID=P\nAGENT=solo\nTYPE=claude\nKIND=worker\nREPO_NAME=repo\nREPO_ROOT=%s\nWORKTREE=%s\nBRANCH=main\nBASE=main\nRUN=1\n' "$HOME/repo" "$HOME/repo" > "$TASKS/P/meta.env"
  printf 'Inspect the cache symbol and its callers.\n' > "$TASKS/P/task.md"
  meta_load P
}

@test "dashboard Serena guidance reaches every role and continuation" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  prompt_fixture
  local role mode out
  export KIND TEAM REVIEW INVESTIGATION_TEAM INVESTIGATION_PHASE FRESH_WHY
  export AGENT_SERENA=/opt/serena/bin/serena
  for role in solo team helper investigation review independent merger independent-review; do
    KIND=worker TEAM='' REVIEW='' INVESTIGATION_TEAM='' INVESTIGATION_PHASE=''
    case $role in
      team) TEAM=T-team ;;
      helper) KIND=helper ;;
      investigation) KIND=investigation ;;
      review) KIND=investigation REVIEW=1 ;;
      independent) KIND=investigation INVESTIGATION_TEAM=I INVESTIGATION_PHASE=1 ;;
      merger) KIND=investigation INVESTIGATION_TEAM=I INVESTIGATION_PHASE=2 ;;
      independent-review) KIND=investigation INVESTIGATION_TEAM=I INVESTIGATION_PHASE=3 ;;
    esac
    meta_set P INVESTIGATION_TEAM "$INVESTIGATION_TEAM"
    for mode in new resume fresh; do
      FRESH_WHY='context limit'
      out=$(build_prompt P "$mode" "$TASKS/P/questions-1.md" 1)
      [[ $out == *'MCP server `serena`'* ]]
      [[ $out != *'{{'* ]]
    done
  done
}

@test "dashboard resume does not advertise Serena when disabled" {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/bin/agent"
  prompt_fixture
  export AGENT_SERENA=''
  out=$(build_prompt P resume "$TASKS/P/questions-1.md" 1)
  [[ $out != *'MCP server `serena`'* ]]
  [[ $out == *'Only the coordinator may open a PR.'* ]]
}
