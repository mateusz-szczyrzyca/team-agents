package main

import (
	_ "embed"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// legacyState is a state.json as written by the version before typed enums (empty tiers, investigations
// without a kind).
//
//go:embed testdata/state-legacy.json
var legacyState []byte

// installState writes a state.json into a fresh bus directory.
func installState(t *testing.T, state []byte) *Bus {
	t.Helper()
	b := newBus(t)
	if err := os.WriteFile(filepath.Join(b.dir, "state.json"), state, 0o600); err != nil {
		t.Fatal(err)
	}
	return b
}

// TestLegacyStateLoads guards compatibility with state.json files written by earlier versions (AGENTS.md,
// rule 9): they must load, keep their meaning, and be saved in the current format.
func TestLegacyStateLoads(t *testing.T) {
	b := installState(t, legacyState)
	mustWith(t, b, func(st *State) {
		old := st.Teams["20261013-080000-team"]
		if old == nil || len(st.Teams) != 2 || len(st.Investigations) != 2 || len(st.Helpers) != 1 {
			t.Fatalf("not everything was loaded: %d teams, %d investigations, %d helpers",
				len(st.Teams), len(st.Investigations), len(st.Helpers))
		}
		if got := old.Members["claude-1"].Tier; got != TierNormal {
			t.Errorf("an empty tier must be read as normal, got %q", got)
		}
		if got := old.Members["openrouter-haiku"].Tier; got != TierWorker {
			t.Errorf("worker tier: got %q", got)
		}
		if got := st.Teams["20261015-101500-team"].Members["codex-1"].State; got != MemberSleeping {
			t.Errorf("member state: got %q", got)
		}
		if st.Investigations["1"].Kind != KindInvestigation || st.Investigations["2"].Kind != KindReview {
			t.Errorf("investigation kinds: %q, %q", st.Investigations["1"].Kind, st.Investigations["2"].Kind)
		}
		if old.Board["B1"].Level != LevelEasy || old.Board["B1"].Status != BoardDone || old.Quiz.Answers["openrouter-haiku"].Secs != 40 {
			t.Errorf("board or quiz not loaded: %+v %+v", old.Board["B1"], old.Quiz)
		}
		if st.Helpers["1"].Status != HelperDone || st.Investigations["2"].Status != InvRejected {
			t.Errorf("statuses: %q %q", st.Helpers["1"].Status, st.Investigations["2"].Status)
		}
	})
	data, err := os.ReadFile(filepath.Join(b.dir, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), `"tier": ""`) || !strings.Contains(string(data), `"tier": "normal"`) {
		t.Fatal("the state must be saved in the current format (empty tiers as normal)")
	}
	// a team operation works on the migrated state
	lead := server(b, "claude-1", func(s *Server) { s.team = "20261015-101500-team" })
	chk(t).ok(toolResult(lead.claimLead()))
}

// TestUnknownStateValueExplainsItself: a value no version ever wrote is still rejected, with a message that
// says what kind of problem it is.
func TestUnknownStateValueExplainsItself(t *testing.T) {
	b := installState(t, []byte(`{"teams":{"T":{"id":"T","members":{"a":{"agent":"a","state":"active","tier":"boss"}}}}}`))
	serr := b.locked(func(*State) {})
	var verr *ValueError
	if serr == nil || !errors.As(serr, &verr) || !strings.Contains(serr.Message, "a value this version does not know") {
		t.Fatalf("got %v", serr)
	}
}
