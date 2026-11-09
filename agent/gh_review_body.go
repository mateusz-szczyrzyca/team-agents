package main

// InsertReviewByline inserts an investigator review byline at the top of a review body.
// If byline is empty, returns body unchanged.
// If the first line (after trimming at most one leading newline) contains
// the marker "Review by the team-agents investigator", returns body unchanged (idempotency).
// Otherwise, prepends the byline with two newlines to the body.
func InsertReviewByline(_, _ string) string {
	return ""
}
