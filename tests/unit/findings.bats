#!/usr/bin/env bats
# findings_count reads how many critical (blocker) and major problems a --verify review reports — in every form an
# investigator writes it. A miss means the team is never woken to fix them.

load helpers

@test "the critical and major findings of a review are counted in every usual form" {
  source "$REPO_ROOT/bin/agent"
  check() { [ "$(findings_count "$1")" = "$2" ] || { echo "findings_count '$1' = $(findings_count "$1"), want $2"; return 1; }; }
  check "Problems: critical 0, major 2, minor 1" 2
  check "Problems: critical: 1, major: 3, minor: 0" 4
  check "Problems: 0 critical, 2 major, 5 minor" 2
  check "**Critical:** 0 · **Major:** 2 · **Minor:** 1" 2
  check "| Critical | 1 |"$'\n'"| Major | 2 |"$'\n'"| Minor | 4 |" 3
  check "Major (2): the parser drops the last line" 2
  check "problems grouped by severity (critical / major / minor); no major problems, minor 2" 0
  check "Problems: critical 0, major 0, minor 3" 0
  check "" 0
}
