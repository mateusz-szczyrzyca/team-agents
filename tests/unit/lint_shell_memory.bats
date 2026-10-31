#!/usr/bin/env bats
# Separate ShellCheck processes release sourced ASTs between files on small hosts.
REPO_ROOT=$(cd "$BATS_TEST_DIRNAME/../.." && pwd)

# setup installs a private fake tool without changing the repository's check inputs.
setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  mkdir -p "$HOME/.local/bin"
  cat > "$HOME/.local/bin/shellcheck" <<'CHECK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$#" >> "$HOME/check-argc"
printf '%s\n' "${@:3}" >> "$HOME/check-files"
for source_file in "${@:3}"; do
  if [[ $source_file == */bad.sh && -f $HOME/reject ]]; then
    exit 1
  fi
done
CHECK
  chmod +x "$HOME/.local/bin/shellcheck"
  touch "$HOME/a.sh" "$HOME/bad.sh" "$HOME/z.sh"
}

@test "lint-shell bounds each process and checks every file before reporting failure" {
  local scripts="$HOME/a.sh $HOME/bad.sh $HOME/z.sh" expected
  expected=$(printf '%s\n' "$HOME/a.sh" "$HOME/bad.sh" "$HOME/z.sh")
  run make -s -C "$REPO_ROOT" lint-shell "SHELL_SCRIPTS=$scripts"
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/check-argc")" = $'3\n3\n3' ]
  [ "$(cat "$HOME/check-files")" = "$expected" ]

  : > "$HOME/check-argc"
  : > "$HOME/check-files"
  touch "$HOME/reject"
  run make -s -C "$REPO_ROOT" lint-shell "SHELL_SCRIPTS=$scripts"
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/check-argc")" = $'3\n3\n3' ]
  [ "$(cat "$HOME/check-files")" = "$expected" ]
}
