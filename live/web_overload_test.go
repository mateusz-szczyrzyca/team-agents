package main

// The Agents view names an overloaded model as the reason of a pause (written first, docs/tdd.md).

import (
	"strings"
	"testing"
)

func TestPausedTextOverloaded(t *testing.T) {
	got := pausedText(map[string]string{"WAKE_AT": "1792735200", "PAUSE_REASON": "overloaded"})
	if !strings.Contains(got, "(model overloaded at the provider)") {
		t.Errorf("pausedText(overloaded) = %q", got)
	}
}
