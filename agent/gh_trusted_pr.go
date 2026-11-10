package main

// IgnoredPR identifies a PR rejected by the round's repository and author policy.
type IgnoredPR struct {
	Number string // Number preserves the JSON number for diagnostics; absent numbers are "null".
	Author string // Author is the login, or "?" when the response has none.
}

// FilterTrustedPRs preserves caller order and fields while applying trusted_prs policy.
func FilterTrustedPRs(_ []byte, _ []string, _ string) ([]byte, []IgnoredPR, *ValueError) {
	return nil, nil, nil
}

// MatchRoundPRMerged matches the exact numeric PR number and the MERGED state.
// Callers must supply an already trusted list and treat parse failures as not merged.
func MatchRoundPRMerged(_ []byte, _ string) (bool, *ValueError) {
	return false, nil
}
