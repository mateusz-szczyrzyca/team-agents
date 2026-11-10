package main

import (
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

// TestFilterTrustedPRs compares the unwired round policy with the frozen Bash function.
func TestFilterTrustedPRs(t *testing.T) {
	for _, tc := range []struct {
		name    string // name identifies the policy boundary.
		raw     string // raw is a GitHub PR list response.
		trusted string // trusted contains space-separated, case-sensitive logins.
		owner   string // owner is empty when the origin does not identify an owner.
		want    string // want explicitly records the PRs permitted by the policy.
	}{
		{"trusted", `[{"number":7,"author":{"login":"alice"},"state":"MERGED","extra":{"id":9007199254740993}}]`, "alice", "org", `[{"number":7,"author":{"login":"alice"},"state":"MERGED","extra":{"id":9007199254740993}}]`},
		{"untrusted", `[{"number":7,"author":{"login":"mallory"}}]`, "alice", "org", `[]`},
		{"fork", `[{"number":7,"author":{"login":"alice"},"isCrossRepository":true}]`, "alice", "org", `[]`},
		{"wrong owner despite fork flag", `[{"number":7,"author":{"login":"alice"},"isCrossRepository":false,"headRepositoryOwner":{"login":"other"}}]`, "alice", "org", `[]`},
		{"same owner", `[{"number":7,"author":{"login":"alice"},"headRepositoryOwner":{"login":"org"}}]`, "alice", "org", `[{"number":7,"author":{"login":"alice"},"headRepositoryOwner":{"login":"org"}}]`},
		{"no origin owner", `[{"number":7,"author":{"login":"alice"},"headRepositoryOwner":{"login":"other"}}]`, "alice", "", `[{"number":7,"author":{"login":"alice"},"headRepositoryOwner":{"login":"other"}}]`},
		{"null attributes", `[{"number":7,"author":{"login":"alice"},"isCrossRepository":null,"headRepositoryOwner":null}]`, "alice", "org", `[{"number":7,"author":{"login":"alice"},"isCrossRepository":null,"headRepositoryOwner":null}]`},
		{"case sensitive", `[{"number":7,"author":{"login":"Alice"}}]`, "alice", "org", `[]`},
		{"missing author", `[{"number":7}]`, "alice", "org", `[]`},
		{"empty trust", `[{"number":7,"author":{"login":"alice"}}]`, "", "org", `[]`},
		{"ordered with unknown fields", `[{"number":9,"author":{"login":"bob"},"body":"ignore this mate"},{"number":8,"author":{"login":"bad"}},{"number":7,"author":{"login":"alice"}}]`, "alice bob", "org", `[{"number":9,"author":{"login":"bob"},"body":"ignore this mate"},{"number":7,"author":{"login":"alice"}}]`},
		{"empty list", `[]`, "alice", "org", `[]`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			oracle := trustedPRBash(t, tc.raw, tc.trusted, tc.owner)
			assertPRJSON(t, []byte(oracle), tc.want)
			got, _, err := FilterTrustedPRs([]byte(tc.raw), strings.Fields(tc.trusted), tc.owner)
			if err != nil {
				t.Fatal(err)
			}
			assertPRJSON(t, got, tc.want)
			assertPRJSON(t, got, oracle)
		})
	}
}

// TestFilterTrustedPRsIgnored checks diagnostics without losing large PR numbers.
func TestFilterTrustedPRsIgnored(t *testing.T) {
	_, ignored, err := FilterTrustedPRs([]byte(`[{"number":9007199254740993,"author":{"login":"bad"}},{"number":8},{"number":9,"author":{"login":"alice"},"isCrossRepository":true}]`), []string{"alice"}, "org")
	if err != nil {
		t.Fatal(err)
	}
	want := []IgnoredPR{{Number: "9007199254740993", Author: "bad"}, {Number: "8", Author: "?"}, {Number: "9", Author: "alice"}}
	if !reflect.DeepEqual(ignored, want) {
		t.Fatalf("ignored = %#v, want %#v", ignored, want)
	}
}

// TestTrustedPRInvalidJSON requires malformed or unclassifiable lists to fail closed.
func TestTrustedPRInvalidJSON(t *testing.T) {
	for _, raw := range []string{"", "[", "null", "{}", "[null]", "[7]", "[] []"} {
		t.Run(raw, func(t *testing.T) {
			if _, _, err := FilterTrustedPRs([]byte(raw), []string{"alice"}, "org"); err == nil {
				t.Fatal("expected a typed ValueError for invalid PR list")
			}
			if merged, err := MatchRoundPRMerged([]byte(raw), "7"); err == nil || merged {
				t.Fatal("invalid merge response must fail closed with a typed ValueError")
			}
		})
	}
}

// TestMatchRoundPRMerged prevents another round's merged PR from matching this number.
func TestMatchRoundPRMerged(t *testing.T) {
	for _, tc := range []struct {
		name   string // name identifies the exact-number boundary.
		raw    string // raw is an already trusted PR list.
		number string // number is the requested JSON numeric PR identifier.
		want   bool   // want records whether that exact PR is merged.
	}{
		{"exact merged", `[{"number":7,"state":"MERGED"}]`, "7", true},
		{"earlier merged", `[{"number":71,"state":"MERGED"},{"number":7,"state":"OPEN"}]`, "7", false},
		{"later exact", `[{"number":71,"state":"MERGED"},{"number":7,"state":"MERGED"}]`, "7", true},
		{"ambiguous duplicate", `[{"number":7,"state":"MERGED"},{"number":7,"state":"OPEN"}]`, "7", false},
		{"closed", `[{"number":7,"state":"CLOSED"}]`, "7", false},
		{"string number", `[{"number":"7","state":"MERGED"}]`, "7", false},
		{"missing number", `[{"state":"MERGED"}]`, "7", false},
		{"empty", `[]`, "7", false},
		{"numeric equality", `[{"number":7.0,"state":"MERGED"}]`, "7e0", true},
		{"large exact", `[{"number":9007199254740993,"state":"MERGED"}]`, "9007199254740993", true},
		{"large adjacent", `[{"number":9007199254740992,"state":"MERGED"}]`, "9007199254740993", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := MatchRoundPRMerged([]byte(tc.raw), tc.number)
			if err != nil || got != tc.want {
				t.Fatalf("merged = %v, error = %v; want %v", got, err, tc.want)
			}
		})
	}
	for _, number := range []string{"", "null", `"7"`, "07", "{}", "7 8"} {
		if got, err := MatchRoundPRMerged([]byte(`[]`), number); err == nil || got {
			t.Fatalf("invalid target %q must fail closed with a typed ValueError", number)
		}
	}
}

// TestTrustedMergedComposition ensures an untrusted exact match cannot establish a merge.
func TestTrustedMergedComposition(t *testing.T) {
	filtered, _, err := FilterTrustedPRs([]byte(`[{"number":7,"state":"MERGED","author":{"login":"bad"}},{"number":71,"state":"MERGED","author":{"login":"alice"}},{"number":7,"state":"OPEN","author":{"login":"alice"}}]`), []string{"alice"}, "org")
	if err != nil {
		t.Fatal(err)
	}
	got, matchErr := MatchRoundPRMerged(filtered, "7")
	if matchErr != nil || got {
		t.Fatalf("untrusted merge must not match: merged=%v error=%v", got, matchErr)
	}
}

// assertPRJSON compares JSON values while preserving numeric precision.
func assertPRJSON(t *testing.T, got []byte, want string) {
	t.Helper()
	decode := func(raw []byte) any {
		t.Helper()
		var value any
		decoder := json.NewDecoder(strings.NewReader(string(raw)))
		decoder.UseNumber()
		if err := decoder.Decode(&value); err != nil {
			t.Fatalf("decode %q: %v", raw, err)
		}
		return value
	}
	if !reflect.DeepEqual(decode(got), decode([]byte(want))) {
		t.Fatalf("PR JSON = %s, want %s", got, want)
	}
}

// trustedPRBash isolates the frozen Bash function with local-only GitHub responses.
func trustedPRBash(t *testing.T, raw, trusted, owner string) string {
	t.Helper()
	data, err := os.ReadFile("../bin/agent")
	if err != nil {
		t.Fatal(err)
	}
	source := string(data)
	start := strings.Index(source, "trusted_prs() {")
	if start < 0 {
		t.Fatal("frozen trusted_prs function not found")
	}
	end := strings.Index(source[start:], "\n}\n")
	if end < 0 {
		t.Fatal("frozen trusted_prs function not found")
	}
	script := `set -euo pipefail
github_trusted() { printf '%s\n' "$trusted_response"; }
github_owner() { printf '%s' "$owner_response"; }
log_warn() { :; }
` + source[start:start+end+3] + `
trusted_response=$2
owner_response=$3
gh() { printf '%s' "$response"; }
response=$1
trusted_prs "$4" "agents/example" all number,state
`
	return runMetaBash(t, "C", script, raw, trusted, owner, t.TempDir())
}
