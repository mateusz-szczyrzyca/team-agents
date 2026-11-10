package main

import (
	"testing"
)

// TestRemoveGeneratedLinesUnicodeGaps tests that RemoveGeneratedLines handles
// emoji separators and trailing whitespace with exact Bash [[:space:]] Unicode semantics.
func TestRemoveGeneratedLinesUnicodeGaps(t *testing.T) {
	tests := []struct {
		name string
		body string
		want string
	}{
		{
			name: "emoji followed by U+2003 em-space separator is removed",
			body: "Body\n🤖\u2003Generated with Claude Code\nEnd",
			want: "Body\nEnd",
		},
		{
			name: "generated line followed by trailing U+2003 em-space is removed",
			body: "Body\nGenerated with Claude Code\u2003\nEnd",
			want: "Body\nEnd",
		},
		{
			name: "emoji with U+2003 and trailing U+2003 are both removed",
			body: "Body\n🤖\u2003Generated with Claude Code\u2003\nEnd",
			want: "Body\nEnd",
		},
		{
			name: "control: emoji followed by U+00A0 non-breaking space is preserved",
			body: "Body\n🤖\u00a0Generated with Claude Code\nEnd",
			want: "Body\n🤖\u00a0Generated with Claude Code\nEnd",
		},
		{
			name: "control: emoji followed by U+2007 figure space is preserved",
			body: "Body\n🤖\u2007Generated with Claude Code\nEnd",
			want: "Body\n🤖\u2007Generated with Claude Code\nEnd",
		},
		{
			name: "control: generated line followed by trailing U+00A0 is preserved",
			body: "Body\nGenerated with Claude Code\u00a0\nEnd",
			want: "Body\nGenerated with Claude Code\u00a0\nEnd",
		},
		{
			name: "control: generated line followed by trailing U+2007 is preserved",
			body: "Body\nGenerated with Claude Code\u2007\nEnd",
			want: "Body\nGenerated with Claude Code\u2007\nEnd",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := RemoveGeneratedLines(tt.body)
			if got != tt.want {
				t.Errorf("RemoveGeneratedLines() = %q, want %q", got, tt.want)
			}
		})
	}
}
