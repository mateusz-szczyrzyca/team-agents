package main

import (
	"bytes"
	"encoding/json"
	"math/big"
)

// RoundPRState is a GitHub pull request lifecycle state.
type RoundPRState string

const (
	RoundPROpen   RoundPRState = "OPEN"   // RoundPROpen identifies an open PR.
	RoundPRClosed RoundPRState = "CLOSED" // RoundPRClosed identifies a closed, unmerged PR.
	RoundPRMerged RoundPRState = "MERGED" // RoundPRMerged identifies a merged PR.
)

// Valid reports whether the state belongs to GitHub's PR lifecycle.
func (RoundPRState) Valid() bool { return false }

// UnmarshalJSON validates the state; errors contain *ValueError.
func (*RoundPRState) UnmarshalJSON([]byte) error { return nil }

// IgnoredPR identifies a PR rejected by the round's repository and author policy.
type IgnoredPR struct {
	Number string // Number preserves the JSON number for diagnostics; absent numbers are "null".
	Author string // Author is the login, or "?" when the response has none.
}

// FilterTrustedPRs preserves caller order and fields while applying trusted_prs policy.
func FilterTrustedPRs(raw []byte, trusted []string, owner string) ([]byte, []IgnoredPR, *ValueError) {
	entries, err := roundPRObjects(raw)
	if err != nil {
		return nil, nil, err
	}
	trustedSet := make(map[string]bool, len(trusted))
	for _, login := range trusted {
		trustedSet[login] = true
	}
	kept := make([]json.RawMessage, 0, len(entries))
	var ignored []IgnoredPR
	for _, entry := range entries {
		var fields map[string]json.RawMessage
		if decodeErr := json.Unmarshal(entry, &fields); decodeErr != nil {
			return nil, nil, valueWrap(decodeErr, "invalid PR object")
		}
		author, authorOK := roundPRLogin(fields["author"], "")
		headOwner, ownerOK := roundPRLogin(fields["headRepositoryOwner"], owner)
		var fork bool
		forkErr := json.Unmarshal(fields["isCrossRepository"], &fork)
		// Missing and null fork flags have the same false default as Bash's jq filter.
		forkOK := len(fields["isCrossRepository"]) == 0 || forkErr == nil
		if authorOK && forkOK && !fork && trustedSet[author] && (owner == "" || (ownerOK && owner == headOwner)) {
			kept = append(kept, entry)
			continue
		}
		number := fields["number"]
		if len(number) == 0 {
			number = json.RawMessage("null")
		}
		author, authorOK = roundPRLogin(fields["author"], "?")
		if !authorOK {
			author = "?"
		}
		ignored = append(ignored, IgnoredPR{Number: string(number), Author: author})
	}
	result, marshalErr := json.Marshal(kept)
	if marshalErr != nil {
		return nil, nil, valueWrap(marshalErr, "encode trusted PRs")
	}
	return result, ignored, nil
}

// MatchRoundPRMerged matches the exact numeric PR number and the MERGED state.
// Callers must supply an already trusted list and treat parse failures as not merged.
func MatchRoundPRMerged(raw []byte, number string) (bool, *ValueError) {
	target := roundPRNumber([]byte(number))
	if target == nil {
		return false, valueErr("invalid round PR number")
	}
	entries, err := roundPRObjects(raw)
	if err != nil {
		return false, err
	}
	matches := 0
	merged := false
	for _, entry := range entries {
		var fields map[string]json.RawMessage
		if decodeErr := json.Unmarshal(entry, &fields); decodeErr != nil {
			return false, valueWrap(decodeErr, "invalid PR object")
		}
		candidate := roundPRNumber(fields["number"])
		if candidate == nil || candidate.Cmp(target) != 0 {
			continue
		}
		matches++
		var state string
		if decodeErr := json.Unmarshal(fields["state"], &state); decodeErr != nil {
			continue
		}
		merged = state == "MERGED"
	}
	return matches == 1 && merged, nil
}

// roundPRObjects validates the response shape before either read-only operation uses it.
func roundPRObjects(raw []byte) ([]json.RawMessage, *ValueError) {
	raw = bytes.TrimSpace(raw)
	if len(raw) == 0 || raw[0] != '[' {
		return nil, valueErr("expected a JSON PR list")
	}
	var entries []json.RawMessage
	if err := json.Unmarshal(raw, &entries); err != nil {
		return nil, valueWrap(err, "invalid PR list")
	}
	for _, entry := range entries {
		if len(entry) == 0 || entry[0] != '{' {
			return nil, valueErr("expected a JSON PR object")
		}
	}
	return entries, nil
}

// roundPRLogin applies jq's missing/null default while preserving an explicit empty login.
func roundPRLogin(raw []byte, fallback string) (string, bool) {
	if len(raw) == 0 || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return fallback, true
	}
	var identity struct {
		Login *string `json:"login"` // Login is nil when missing or null.
	}
	if err := json.Unmarshal(raw, &identity); err != nil {
		return "", false
	}
	if identity.Login == nil {
		return fallback, true
	}
	return *identity.Login, true
}

// roundPRNumber compares JSON numbers exactly, without a float64 precision loss.
func roundPRNumber(raw []byte) *big.Rat {
	raw = bytes.TrimSpace(raw)
	if len(raw) == 0 || (raw[0] != '-' && (raw[0] < '0' || raw[0] > '9')) || !json.Valid(raw) {
		return nil
	}
	number, ok := new(big.Rat).SetString(string(raw))
	if !ok {
		return nil
	}
	return number
}
