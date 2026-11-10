package main

import (
	"encoding/json"
	"errors"
	"testing"
)

// TestRoundPRStateJSON requires known states to round-trip and invalid states to fail typed.
func TestRoundPRStateJSON(t *testing.T) {
	for _, state := range []RoundPRState{RoundPROpen, RoundPRClosed, RoundPRMerged} {
		var decoded RoundPRState
		if err := json.Unmarshal([]byte(`"`+string(state)+`"`), &decoded); err != nil || !state.Valid() || decoded != state {
			t.Fatalf("state %q: decoded=%q error=%v", state, decoded, err)
		}
		encoded, err := json.Marshal(decoded)
		if err != nil || string(encoded) != `"`+string(state)+`"` {
			t.Fatalf("encode %q: %s, %v", state, encoded, err)
		}
	}
	for _, raw := range []string{`"UNKNOWN"`, `"merged"`, `null`, `7`} {
		var state RoundPRState
		var failure *ValueError
		if err := json.Unmarshal([]byte(raw), &state); !errors.As(err, &failure) {
			t.Fatalf("state %s must return ValueError: %v", raw, err)
		}
		if merged, err := MatchRoundPRMerged([]byte(`[{"number":7,"state":`+raw+`}]`), "7"); err != nil || merged {
			t.Fatalf("invalid state merged=%v error=%v", merged, err)
		}
	}
}

// TestTrustedPRExactLoginKeys preserves jq's case-sensitive object key lookup.
func TestTrustedPRExactLoginKeys(t *testing.T) {
	for _, raw := range []string{
		`[{"author":{"Login":"alice"}}]`,
		`[{"author":{"LOGIN":"alice"}}]`,
		`[{"author":{"login":"mallory","Login":"alice"}}]`,
		`[{"author":{"login":"alice"},"headRepositoryOwner":{"login":"other","Login":"org"}}]`,
	} {
		got, _, err := FilterTrustedPRs([]byte(raw), []string{"alice"}, "org")
		if err != nil {
			t.Fatal(err)
		}
		assertPRJSON(t, got, "[]")
		assertPRJSON(t, got, trustedPRBash(t, raw, "alice", "org"))
	}
}

// TestRoundPRJSONWhitespace rejects framing excluded from the JSON grammar.
func TestRoundPRJSONWhitespace(t *testing.T) {
	for _, space := range []string{"\v", "\f", "\u00a0"} {
		raw := []byte(space + `[{"number":7,"state":"MERGED"}]` + space)
		if _, _, err := FilterTrustedPRs(raw, nil, ""); err == nil {
			t.Fatalf("accepted response framing %q", space)
		}
		if merged, err := MatchRoundPRMerged(raw, "7"); err == nil || merged {
			t.Fatalf("accepted merge response framing %q", space)
		}
		if merged, err := MatchRoundPRMerged([]byte(`[{"number":7,"state":"MERGED"}]`), space+"7"+space); err == nil || merged {
			t.Fatalf("accepted target framing %q", space)
		}
	}
	if merged, err := MatchRoundPRMerged([]byte(" \t\r\n"+`[{"number":7,"state":"MERGED"}]`+"\n"), " \t7\r\n"); err != nil || !merged {
		t.Fatalf("JSON whitespace: merged=%v error=%v", merged, err)
	}
}
