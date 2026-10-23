#!/usr/bin/env bats

load helpers

# detector_setup loads only function definitions using the isolated test configuration.
detector_setup() {
  source "$REPO_ROOT/bin/agent"
  mkdir -p "$TASKS/detector"
  printf 'WORKTREE=%q\n' "$HOME/repo" > "$TASKS/detector/meta.env"
}

@test "pending detector ignores assertions comments and string fixtures" {
  detector_setup
  cat > assertions.bats <<'FIXTURE'
  grep -q "pending: parser, round 1" "$HOME/prompt-solo-resume.txt"
# pending: documented example, round 1
expected='t.Skip("pending: string fixture, round 1")'
printf '%s\n' 'pending: output fixture, round 1'
FIXTURE
  git add assertions.bats
  run pending_due detector 1
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "pending detector retains due Go and Bats skip statements with original locations" {
  detector_setup
  cat > parser_test.go <<'FIXTURE'
package parser
func TestParser(t *testing.T) {
  t.Skip("pending: parser, round ONE")
  t.Skipf("pending: formatting, round TWO")
  t.Skip("pending: future parser, round THREE")
}
FIXTURE
  cat > parser.bats <<'FIXTURE'
  skip "pending: shell parser, round TWO"
FIXTURE
  # Substitute fixture rounds after writing so sample skips are not pending tests in this file.
  sed -i -e s/ONE/1/g -e s/TWO/2/g -e s/THREE/3/g parser_test.go parser.bats
  git add parser_test.go parser.bats
  run pending_due detector 2
  [ "$status" -eq 0 ]
  [ "$output" = $'parser.bats:1:  skip "pending: shell parser, round 2"\nparser_test.go:3:  t.Skip("pending: parser, round 1")\nparser_test.go:4:  t.Skipf("pending: formatting, round 2")' ]
}

@test "pending detector without a due round includes all skips and ignores Markdown examples" {
  detector_setup
  printf '  t.Skip("pending: future parser, round 9")\n' > parser_test.go
  printf '  t.Skip("pending: documentation, round 1")\n' > example.md
  git add parser_test.go example.md
  run pending_due detector
  [ "$status" -eq 0 ]
  [ "$output" = 'parser_test.go:1:  t.Skip("pending: future parser, round 9")' ]
}
