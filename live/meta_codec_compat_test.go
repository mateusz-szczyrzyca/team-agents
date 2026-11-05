package main

import "testing"

// TestMetaCodecControlEscapes reads the exact candidate/Bash golden bytes across modules.
func TestMetaCodecControlEscapes(t *testing.T) {
	m := readMeta("../agent/testdata/meta-live.env")
	for key, want := range map[string]string{"EMPTY": "", "TEXT": "hello world", "QUOTE": "quo'te\\slash", "UNICODE": "zażółć", "CONTROLS": "\a\b\t\n\v\f\r\x1b"} {
		if got, ok := m[key]; !ok || got != want {
			t.Errorf("%s decoded to %q,%v want %q", key, got, ok, want)
		}
	}
}
