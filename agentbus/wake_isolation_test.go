package main

// Tests never wake a real task: spawnWake runs `agent _wake <task>` on the machine that runs the tests — an
// agent running make test on the host woke (or failed to wake) real tasks with the fixtures' IDs.

import (
	"reflect"
	"testing"
)

func TestTestsNeverWakeRealTasks(t *testing.T) {
	if reflect.ValueOf(wakeFn).Pointer() == reflect.ValueOf(spawnWake).Pointer() {
		t.Fatal("wakeFn is spawnWake in the tests: a test that puts a member to sleep wakes a real task")
	}
}
