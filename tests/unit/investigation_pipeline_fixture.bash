#!/usr/bin/env bash
# Successful isolated investigation pipeline fixture shared by singleton tests.

# single_pipeline_setup installs isolated phase-aware CLIs and a local GitHub remote substitute.
single_pipeline_setup() {
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-cli" "$HOME/.local/bin/independent-fixture"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-pipeline-cli" "$HOME/.local/bin/claude"
  printf '\nAGENT_REVIEW_GATE=true\nAGENT_STAGE_POLL_SEC=1\n' >> "$AGENTS_HOME/agents.conf"
  git config url."$HOME/remote.git".insteadOf https://github.com/example/repo.git
  git remote set-url origin https://github.com/example/repo.git
  export PIPELINE_REAL_GIT
  PIPELINE_REAL_GIT=$(command -v git)
  mkdir -p "$HOME/pipeline-bin"
  install -m755 "$BATS_TEST_DIRNAME/stubs/fake-investigation-git" "$HOME/pipeline-bin/git"
  export PATH="$HOME/pipeline-bin:$PATH"
  printf '\nRUN_PATH=%q\n' "$PATH" >> "$AGENTS_HOME/agents.conf"
}
