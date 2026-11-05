package main

import (
	"bytes"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// TestMetaLiteralRecords covers legacy encodings and additive hand-written fixture forms.
func TestMetaLiteralRecords(t *testing.T) {
	cases := map[string]string{
		"": "", "''": "", "plain": "plain", `one\ two`: "one two",
		`quo\'te\\end`: "quo'te\\end", `'a b'`: "a b", `"a\\b\$c"`: "a\\b$c",
		`a'b c'$'\n'`: "ab c\n", `$'\a\b\t\n\v\f\r\E'`: "\a\b\t\n\v\f\r\x1b",
		`$'\101\x42\u0043\U00000044\cE'`: "ABCD\x05", `$'\q'`: `\q`,
		`$'a\0ignored'after`: "aafter", `$'\303\251'`: "é", "zażółć": "zażółć",
		"foo\\\nbar": "foobar", "'line1\nline2'": "line1\nline2", "value # comment": "value",
		`dollar$`: "dollar$", "value\r": "value\r",
	}
	for encoded, want := range cases {
		t.Run(encoded, func(t *testing.T) {
			data := []byte("FUTURE_FIELD=" + encoded + "\n")
			m := parseMeta(data)
			got, ok := m.Lookup("FUTURE_FIELD")
			if !ok || got != want || !bytes.Equal(m.Bytes(), data) {
				t.Fatalf("decoded %q = %q,%v, want %q; records=%+v", encoded, got, ok, want, m.Lines)
			}
		})
	}
}

// TestMetaPreservesOpaqueLines requires explicit classification instead of executing shell.
func TestMetaPreservesOpaqueLines(t *testing.T) {
	for _, raw := range []string{`KEY=$HOME`, `KEY=$(touch marker)`, "KEY=`id`", `KEY=~/x`, `KEY=a b`, `KEY=x; echo y`, `export KEY=x`, `KEY=(a b)`, `KEY='unfinished`} {
		m := parseMeta([]byte(raw))
		if len(m.Lines) != 1 || m.Lines[0].Kind != MetaShell || string(m.Bytes()) != raw {
			t.Fatalf("unsupported record lost: %q -> %+v", raw, m)
		}
		if _, ok := m.Lookup("KEY"); ok {
			t.Fatalf("shell record exposed as literal: %q", raw)
		}
	}
	data := []byte("# future format\n\nFUTURE=old\n  FUTURE='new value'\nEMPTY=''\nopaque extension")
	m := parseMeta(data)
	if got, ok := m.Lookup("FUTURE"); !ok || got != "new value" {
		t.Fatalf("last duplicate lost: %q,%v", got, ok)
	}
	if got, ok := m.Lookup("EMPTY"); !ok || got != "" {
		t.Fatal("explicit emptiness lost")
	}
	if _, ok := m.Lookup("ABSENT"); ok || !bytes.Equal(m.Bytes(), data) {
		t.Fatal("absence or unknown-line preservation changed")
	}
	if len(m.Lines) != 6 || m.Lines[0].Kind != MetaComment || m.Lines[1].Kind != MetaBlank || m.Lines[2].Encoded != "old" {
		t.Fatalf("record metadata changed: %+v", m.Lines)
	}
	if len(parseMeta(nil).Lines) != 0 || !MetaShell.Valid() || MetaLineKind(99).Valid() {
		t.Fatal("empty file or classification validity changed")
	}
}

// TestMetaBashDifferential checks exact writer bytes and decoded data in both legacy locales.
func TestMetaBashDifferential(t *testing.T) {
	values := []string{"", "plain", "a b", "#hash", "a#hash", "~home", "x=~home:~user", "x~tail", "!\"$&'()*,;<>?[]^`{|}\\", "zażółć é", "line1\nline2\t\r", "\a\b\f\v\x1b\x7f", "\xff\xc3x", "\u200b\u202e\ufeff\ue000", "\u2028\u2029\u0085"}
	for i := 1; i < 128; i++ {
		values = append(values, string(byte(i)))
	}
	for _, value := range values {
		encoded, failure := quoteMeta(value)
		if failure != nil {
			t.Fatal(failure)
		}
		want := runMetaBash(t, "C.UTF-8", `printf '%q' "$1"`, value)
		if encoded != want {
			t.Fatalf("encoder(%q)=%q, Bash=%q", value, encoded, want)
		}
		for _, locale := range []string{"C", "C.UTF-8"} {
			legacy := runMetaBash(t, locale, `printf '%q' "$1"`, value)
			m := parseMeta([]byte("VALUE=" + legacy + "\n"))
			if got, ok := m.Lookup("VALUE"); !ok || got != value {
				t.Fatalf("%s decode %q -> %q,%v want %q", locale, legacy, got, ok, value)
			}
		}
		record, failure := formatMetaAssignment("VALUE", value)
		if failure != nil {
			t.Fatal(failure)
		}
		file := filepath.Join(t.TempDir(), "meta.env")
		writeMetaFixture(t, file, record)
		if got := runMetaBash(t, "C", `source "$1"; printf '%s' "$VALUE"`, file); got != value {
			t.Fatalf("Bash source(%q)=%q want %q", record, got, value)
		}
	}
}

// TestMetaInvalidAssignments rejects values Bash variables cannot retain.
func TestMetaInvalidAssignments(t *testing.T) {
	for _, key := range []string{"", "1KEY", "A-B", "X\nY", "X[0]", "A;touch marker"} {
		if _, failure := formatMetaAssignment(key, "value"); failure == nil || !errors.Is(failure.asError(), ErrInvalidArgument) {
			t.Fatalf("invalid key %q accepted: %v", key, failure)
		}
	}
	if _, failure := quoteMeta("a\x00b"); failure == nil || !errors.Is(failure.asError(), ErrInvalidArgument) {
		t.Fatal("NUL value accepted")
	}
}

// TestMetaLiveGolden fixes writer bytes consumed by the current live-reader regression.
func TestMetaLiveGolden(t *testing.T) {
	data, failure := os.ReadFile("testdata/meta-live.env")
	if failure != nil {
		t.Fatal(failure)
	}
	var output strings.Builder
	for _, field := range [][2]string{{"EMPTY", ""}, {"TEXT", "hello world"}, {"QUOTE", "quo'te\\slash"}, {"UNICODE", "zażółć"}, {"CONTROLS", "\a\b\t\n\v\f\r\x1b"}} {
		record, failure := formatMetaAssignment(field[0], field[1])
		if failure != nil {
			t.Fatal(failure)
		}
		output.WriteString(record)
	}
	if output.String() != string(data) {
		t.Fatalf("candidate writer differs from live golden: %q", output.String())
	}
}

// TestReadTaskMeta distinguishes missing files, storage failures, and optional fields.
func TestReadTaskMeta(t *testing.T) {
	tasks := t.TempDir()
	if _, failure := readTaskMeta(tasks, "missing"); failure == nil || failure.Cause != ReadTaskMetaNotFound {
		t.Fatalf("missing cause = %v", failure)
	}
	file := filepath.Join(tasks, "task", "meta.env")
	if failure := os.MkdirAll(filepath.Dir(file), 0o700); failure != nil {
		t.Fatal(failure)
	}
	writeMetaFixture(t, file, "STATUS=historic_unknown\nRUN=not-a-number\nFUTURE=value\nNOTE="+strings.Repeat("x", 100000))
	m, failure := readTaskMeta(tasks, "task")
	if failure != nil {
		t.Fatal(failure)
	}
	if value, ok := m.Lookup("RUN"); !ok || value != "not-a-number" {
		t.Fatal("legacy field was validated as an enum or number")
	}
	if value, ok := m.Lookup("NOTE"); !ok || len(value) != 100000 {
		t.Fatal("large record truncated")
	}
	if failure := os.Remove(file); failure != nil {
		t.Fatal(failure)
	}
	if failure := os.Mkdir(file, 0o700); failure != nil {
		t.Fatal(failure)
	}
	if _, failure := readTaskMeta(tasks, "task"); failure == nil || failure.Cause != ReadTaskMetaStorage {
		t.Fatalf("read-directory cause = %v", failure)
	}
}

// runMetaBash isolates all oracle evaluations from installation configuration and startup hooks.
func runMetaBash(t *testing.T, locale, script string, args ...string) string {
	t.Helper()
	cmd := exec.Command("bash", append([]string{"--noprofile", "--norc", "-c", script, "meta-test"}, args...)...) // #nosec G204 -- scripts and argv are synthetic test literals, with isolated HOME and startup environment
	cmd.Env = []string{"HOME=" + t.TempDir(), "PATH=/usr/bin:/bin", "LC_ALL=" + locale}
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	output, failure := cmd.Output()
	if failure != nil {
		t.Fatalf("Bash oracle failed: %v: %s", failure, stderr.String())
	}
	return string(output)
}

// writeMetaFixture fails explicitly on storage errors in synthetic legacy fixtures.
func writeMetaFixture(t *testing.T, file, data string) {
	t.Helper()
	if failure := os.WriteFile(file, []byte(data), 0o600); failure != nil {
		t.Fatal(failure)
	}
}
