package main

import (
	"os"
	"testing"
)

// TestMain: no test wakes a real task — spawnWake runs `agent _wake <task>` on the machine that runs the tests.
// Tests that check wake-ups install their own recorder (see agentbus_test.go).
func TestMain(m *testing.M) {
	wakeFn = func(*Team, string) {}
	remoteMemberFn = func(*Team, string, string, string, string) {} // nor runs agent _remote_member
	os.Exit(m.Run())
}
