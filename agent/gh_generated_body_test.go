package main

import (
	"testing"
)

func TestRemoveGeneratedLines(t *testing.T) {
	tests := []struct {
		name string
		body string
		want string
	}{
		{
			name: "no generated line",
			body: "This is a normal body\nwith multiple lines",
			want: "This is a normal body\nwith multiple lines",
		},
		{
			name: "generated line at end",
			body: "Body text\nGenerated with Claude Code",
			want: "Body text",
		},
		{
			name: "generated line with emoji",
			body: "Body text\n🤖 Generated with Claude Code",
			want: "Body text",
		},
		{
			name: "generated line with brackets",
			body: "Body text\nGenerated with [Claude Code]",
			want: "Body text",
		},
		{
			name: "generated line with parentheses",
			body: "Body text\nGenerated with Claude Code(v1.0)",
			want: "Body text",
		},
		{
			name: "generated line with spaces around",
			body: "Body text\n  Generated with Claude Code  ",
			want: "Body text",
		},
		{
			name: "generated line with blank line after",
			body: "Body text\nGenerated with Claude Code\n\nMore text",
			want: "Body text\nMore text",
		},
		{
			name: "generated line with whitespace-only line after",
			body: "Body text\nGenerated with Claude Code\n  \nMore text",
			want: "Body text\nMore text",
		},
		{
			name: "generated line in middle",
			body: "Start\nGenerated with Claude Code\nEnd",
			want: "Start\nEnd",
		},
		{
			name: "multiple generated lines",
			body: "Start\nGenerated with Claude Code\nMiddle\nGenerated with Claude Code\nEnd",
			want: "Start\nMiddle\nEnd",
		},
		{
			name: "generated line with all variations",
			body: "Body\n🤖 Generated with [Claude Code](v1.0)\nEnd",
			want: "Body\nEnd",
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
