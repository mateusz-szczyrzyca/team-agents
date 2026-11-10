package main

import (
	"testing"
)

// TestEmojiWithUnicodeWhitespace tests that emoji followed by Unicode spaces (not just ASCII)
// is recognized as the Generated line marker. This tests the emoji-separator defect where
// the regex pattern accepts only ASCII spaces ([ \t\v\f\r]*) after the emoji.
func TestEmojiWithUnicodeWhitespace(t *testing.T) {
	tests := []struct {
		name string
		body string
		want string
	}{
		{
			name: "emoji with em-space (U+2003) separator",
			body: "Text\n🤖 Generated with Claude Code\nEnd",
			want: "Text\nEnd",
		},
		{
			name: "emoji with multiple em-spaces",
			body: "Text\n🤖   Generated with Claude Code\nEnd",
			want: "Text\nEnd",
		},
		{
			name: "emoji with mixed em-space and ASCII space",
			body: "Text\n🤖  Generated with Claude Code\nEnd",
			want: "Text\nEnd",
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

// TestTrailingUnicodeWhitespace tests that trailing Unicode spaces are correctly trimmed
// before regex matching. This tests the trailing Unicode defect where byte-by-byte iteration
// in trimBashSpaces doesn't correctly handle multi-byte UTF-8 characters.
func TestTrailingUnicodeWhitespace(t *testing.T) {
	tests := []struct {
		name string
		body string
		want string
	}{
		{
			name: "trailing em-space (U+2003) after Generated line",
			body: "Text\nGenerated with Claude Code \nEnd",
			want: "Text\nEnd",
		},
		{
			name: "trailing em-space with bracket suffix",
			body: "Text\nGenerated with [Claude Code] \nEnd",
			want: "Text\nEnd",
		},
		{
			name: "multiple trailing em-spaces",
			body: "Text\nGenerated with Claude Code   \nEnd",
			want: "Text\nEnd",
		},
		{
			name: "mixed trailing em-space and ASCII space",
			body: "Text\nGenerated with Claude Code  \nEnd",
			want: "Text\nEnd",
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
