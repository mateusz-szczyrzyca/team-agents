package main

import (
	"testing"
)

// TestRemoveGeneratedLinesUnicodeParity tests that RemoveGeneratedLines matches
// Bash's gh_publish_text behavior with Unicode spaces and exact whitespace matching.
func TestRemoveGeneratedLinesUnicodeParity(t *testing.T) {
	tests := []struct {
		name string
		body string
		want string
	}{
		{
			name: "generated with U+2003 em-space before text",
			body: "Body\n Generated with Claude Code\nEnd",
			want: "Body\nEnd",
		},
		{
			name: "generated with U+00A0 non-breaking space is not removed",
			body: "Body\n Generated with Claude Code\nEnd",
			want: "Body\n Generated with Claude Code\nEnd",
		},
		{
			name: "blank line with U+00A0 after generated (Bash preserves it)",
			body: "Body\nGenerated with Claude Code\n \nEnd",
			want: "Body\n \nEnd",
		},
		{
			name: "consecutive generated lines with blank between",
			body: "Start\nGenerated with Claude Code\n\nGenerated with Claude Code\nEnd",
			want: "Start\nEnd",
		},
		{
			name: "final newline preserved",
			body: "Body\nGenerated with Claude Code\n",
			want: "Body",
		},
		{
			name: "empty body",
			body: "",
			want: "",
		},
		{
			name: "only generated line",
			body: "Generated with Claude Code",
			want: "",
		},
		{
			name: "generated with mixed emoji and brackets",
			body: "Text\n🤖 Generated with [Claude Code](v1.0)\nMore",
			want: "Text\nMore",
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

// TestBashBlankLineDetection tests that blank line detection uses Bash's [[:space:]] semantics.
func TestBashBlankLineDetection(t *testing.T) {
	tests := []struct {
		line  string
		blank bool
	}{
		{"", true},
		{" ", true},
		{"\t", true},
		{"\n", true},
		{"  \t  ", true},
		{"text", false},
		{" text ", false},
		{" ", false},   // NBSP (U+00A0) is not [[:space:]] in Bash
		{" ", true},    // EM-space (U+2003) IS [[:space:]] in Bash, so blank
		{"   ", false}, // Mixed ASCII and NBSP = not blank (NBSP ruins it)
	}

	for _, tt := range tests {
		t.Run(tt.line, func(t *testing.T) {
			got := isBashBlankLine(tt.line)
			if got != tt.blank {
				t.Errorf("isBashBlankLine(%q) = %v, want %v", tt.line, got, tt.blank)
			}
		})
	}
}
