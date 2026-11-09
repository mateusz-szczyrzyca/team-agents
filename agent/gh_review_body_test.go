package main

import (
	"testing"
)

// TestDoubleInsertReviewBylineIdempotent tests actual repeated application.
func TestDoubleInsertReviewBylineIdempotent(t *testing.T) {
	byline := "Review by the team-agents investigator `inv-test` (investigation, model claude), request #99."
	body := "Original body content"

	// First application should insert the byline.
	after1 := InsertReviewByline(body, byline)
	expected1 := "Review by the team-agents investigator `inv-test` (investigation, model claude), request #99.\n\nOriginal body content"
	if after1 != expected1 {
		t.Errorf("First InsertReviewByline = %q, want %q", after1, expected1)
	}

	// Second application should not insert again (idempotent).
	after2 := InsertReviewByline(after1, byline)
	if after2 != after1 {
		t.Errorf("Second InsertReviewByline changed result: got %q, want %q (unchanged)", after2, after1)
	}
}

// TestResolveAndInsertReviewByline tests metadata-derived byline insertion (ledger 474).
func TestResolveAndInsertReviewByline(t *testing.T) {
	testCases := []struct {
		name     string
		meta     MetaFile
		body     string
		wantBody string
	}{
		{
			name:     "investigation with agent, type, model, and inv_id",
			meta:     parseMeta([]byte("KIND=investigation\nAGENT=inv-claude\nTYPE=investigation\nMODEL=claude-opus-5.5\nINV_ID=42\n")),
			body:     "Some review feedback",
			wantBody: "Review by the team-agents investigator `inv-claude` (investigation, model claude-opus-5.5), request #42.\n\nSome review feedback",
		},
		{
			name:     "investigation with agent and type only",
			meta:     parseMeta([]byte("KIND=investigation\nAGENT=inv-codex\nTYPE=investigation\nINV_ID=1\n")),
			body:     "Feedback here",
			wantBody: "Review by the team-agents investigator `inv-codex` (investigation), request #1.\n\nFeedback here",
		},
		{
			name:     "non-investigation should return empty byline",
			meta:     parseMeta([]byte("KIND=worker\nAGENT=test-worker\nTYPE=test\n")),
			body:     "Some content",
			wantBody: "Some content",
		},
		{
			name:     "idempotent with resolved byline",
			meta:     parseMeta([]byte("KIND=investigation\nAGENT=inv-test\nTYPE=investigation\nMODEL=test-model\nINV_ID=99\n")),
			body:     "Review by the team-agents investigator `inv-test` (investigation, model test-model), request #99.\n\nExisting content",
			wantBody: "Review by the team-agents investigator `inv-test` (investigation, model test-model), request #99.\n\nExisting content",
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			byline := ResolveReviewByline(tc.meta)
			got := InsertReviewByline(tc.body, byline)
			if got != tc.wantBody {
				t.Errorf("ResolveReviewByline+InsertReviewByline = %q, want %q", got, tc.wantBody)
			}
		})
	}
}

// TestInsertReviewByline tests the review body byline insertion.
func TestInsertReviewByline(t *testing.T) {
	testCases := []struct {
		name     string
		body     string
		byline   string
		wantBody string
	}{
		{
			name:     "empty body with byline",
			body:     "",
			byline:   "Review by the team-agents investigator `inv` (type, model X), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type, model X), request #1.\n\n",
		},
		{
			name:     "body with no leading newline",
			body:     "Some feedback here.",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nSome feedback here.",
		},
		{
			name:     "body with one leading newline",
			body:     "\nSome feedback here.",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\n\nSome feedback here.",
		},
		{
			name:     "body with two leading newlines",
			body:     "\n\nSome feedback here.",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\n\n\nSome feedback here.",
		},
		{
			name:     "multiline body",
			body:     "Line 1\nLine 2\nLine 3",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nLine 1\nLine 2\nLine 3",
		},
		{
			name:     "idempotent: marker at start of first line",
			body:     "Review by the team-agents investigator `inv` (type), request #1.\n\nOther content",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nOther content",
		},
		{
			name:     "idempotent: marker mid-first line",
			body:     "Preliminary comment; Review by the team-agents investigator `inv` (type), request #1. ends here",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Preliminary comment; Review by the team-agents investigator `inv` (type), request #1. ends here",
		},
		{
			name:     "marker in second line should insert",
			body:     "Other line\nReview by the team-agents investigator `inv` (type), request #1.",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nOther line\nReview by the team-agents investigator `inv` (type), request #1.",
		},
		{
			name:     "empty byline with body",
			body:     "Some feedback.",
			byline:   "",
			wantBody: "Some feedback.",
		},
		{
			name:     "footer preserved",
			body:     "Review comment\n\nFooter text",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nReview comment\n\nFooter text",
		},
		{
			name:     "one leading newline with marker in first line",
			body:     "\nReview by the team-agents investigator `inv` (type), request #1.\nOther content",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "\nReview by the team-agents investigator `inv` (type), request #1.\nOther content",
		},
		{
			name:     "two leading newlines prepends byline",
			body:     "\n\nReview by the team-agents investigator `inv` (type), request #1.",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\n\n\nReview by the team-agents investigator `inv` (type), request #1.",
		},
		{
			name:     "repeated application idempotent",
			body:     "Review by the team-agents investigator `inv` (type), request #1.\n\nOriginal body",
			byline:   "Review by the team-agents investigator `inv` (type), request #1.",
			wantBody: "Review by the team-agents investigator `inv` (type), request #1.\n\nOriginal body",
		},
		{
			name:     "metadata-derived byline with model",
			body:     "Test feedback",
			byline:   "Review by the team-agents investigator `inv-model` (investigation, model claude), request #123.",
			wantBody: "Review by the team-agents investigator `inv-model` (investigation, model claude), request #123.\n\nTest feedback",
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			got := InsertReviewByline(tc.body, tc.byline)
			if got != tc.wantBody {
				t.Errorf("InsertReviewByline(%q, %q) = %q, want %q", tc.body, tc.byline, got, tc.wantBody)
			}
		})
	}
}
