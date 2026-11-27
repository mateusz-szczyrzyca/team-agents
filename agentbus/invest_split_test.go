package main

import (
	_ "embed"
	"testing"
)

// stateBeforeSplitKind preserves question and review requests written before split requests.
//
//go:embed testdata/state-before-split-kind.json
var stateBeforeSplitKind []byte

func TestOldStateWithoutSplitKindLoads(t *testing.T) {
	b := installState(t, stateBeforeSplitKind)
	for range 2 {
		mustWith(t, b, func(st *State) {
			if len(st.Investigations) != 2 || st.Investigations["1"].Kind != KindInvestigation || st.Investigations["2"].Kind != KindReview {
				t.Fatalf("legacy requests changed: %+v", st.Investigations)
			}
		})
	}
}
