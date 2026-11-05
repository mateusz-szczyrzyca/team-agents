package main

import "testing"

// TestMetaReviewOpaqueExpansions guards syntax whose bytes cannot be decoded literally.
func TestMetaReviewOpaqueExpansions(t *testing.T) {
	for _, raw := range []string{`KEY=$[1+1]`, `KEY="$[1+1]"`, `KEY=$'\uD800'`, `KEY=$'\U00110000'`, `KEY=$'\UFFFFFFFF'`} {
		t.Run(raw, func(t *testing.T) {
			m := parseMeta([]byte(raw))
			if len(m.Lines) != 1 || m.Lines[0].Kind != MetaShell || string(m.Bytes()) != raw {
				t.Fatalf("record must remain opaque and byte-identical: %+v", m)
			}
			if value, ok := m.Lookup("KEY"); ok {
				t.Fatalf("opaque record exposed as literal: %q", value)
			}
		})
	}
}

// TestMetaReviewCommentBoundaries preserves later records after operator comments.
func TestMetaReviewCommentBoundaries(t *testing.T) {
	for _, prefix := range []string{
		"BAD=x;# don't\n",
		"BAD=x\\;#'first\nsecond'\n",
		"BAD=x\\ #\"first\nsecond\"\n",
		"BAD='x;#first\nsecond'\n",
	} {
		t.Run(prefix, func(t *testing.T) {
			raw := prefix + "STATUS=done\n"
			m := parseMeta([]byte(raw))
			if len(m.Lines) != 2 || string(m.Bytes()) != raw {
				t.Fatalf("record boundary or raw bytes changed: %+v", m)
			}
			if value, ok := m.Lookup("STATUS"); !ok || value != "done" {
				t.Fatalf("later assignment lost: %q,%v", value, ok)
			}
		})
	}
}

// TestMetaReviewOpaqueShadow prevents a known shell assignment exposing a stale literal.
func TestMetaReviewOpaqueShadow(t *testing.T) {
	for _, raw := range []string{"KEY=old\nKEY=$(echo new)\n", "KEY=old\nKEY=$[1+1]\n"} {
		m := parseMeta([]byte(raw))
		if value, ok := m.Lookup("KEY"); ok || value != "" {
			t.Fatalf("opaque assignment must hide earlier literal: %q,%v", value, ok)
		}
		if string(m.Bytes()) != raw {
			t.Fatal("shadowing changed preserved bytes")
		}
		m = parseMeta([]byte(raw + "KEY=final\n"))
		if value, ok := m.Lookup("KEY"); !ok || value != "final" {
			t.Fatalf("later literal must supersede opaque assignment: %q,%v", value, ok)
		}
	}
}
