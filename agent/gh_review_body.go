package main

import "strings"

// InsertReviewByline inserts an investigator review byline at the top of a review body.
// If byline is empty, returns body unchanged.
// If the first line (after trimming at most one leading newline) contains
// the marker "Review by the team-agents investigator", returns body unchanged (idempotency).
// Otherwise, prepends the byline with two newlines to the body.
func InsertReviewByline(body, byline string) string {
	if byline == "" {
		return body
	}

	// Trim at most one leading newline.
	trimmed := strings.TrimPrefix(body, "\n")

	// Get the first line.
	var firstLine string
	if idx := strings.Index(trimmed, "\n"); idx != -1 {
		firstLine = trimmed[:idx]
	} else {
		firstLine = trimmed
	}

	// Check idempotency: if the marker is already in the first line, return unchanged.
	if strings.Contains(firstLine, "Review by the team-agents investigator") {
		return body
	}

	// Prepend byline with two newlines.
	return byline + "\n\n" + body
}
