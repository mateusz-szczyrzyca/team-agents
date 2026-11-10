package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Rule: attribution preserves Bash footer bytes, metadata order and raw matching.
func TestFormatAttributionBlock(t *testing.T) {
	config := Config{Associative: map[string]map[string]string{
		"AGENT_TYPE":          {"lead": "codex", "member": "claude"},
		"AGENT_MODEL":         {"lead": "model-a", "member": "model-b"},
		"INVESTIGATORS_TYPE":  {"reviewer": "codex"},
		"INVESTIGATORS_MODEL": {"reviewer": "review-model"},
	}}
	tests := []struct {
		// name identifies the footer rule exercised by this case.
		name string
		// indent prefixes every output line literally.
		indent string
		// agent is the current coordinator or standalone agent.
		agent string
		// team selects exact raw TEAM records; empty means standalone.
		team string
		// records are meta.env contents in task-path order.
		records []string
	}{
		{name: "standalone ignores tasks", agent: "lead", records: []string{"TEAM=t\nAGENT=member\n"}},
		{name: "empty identity", indent: "  "},
		{name: "team without task files", agent: "lead", team: "t"},
		{name: "indented team", indent: "  ", agent: "lead", team: "t", records: []string{
			"TEAM=t\nAGENT=lead\n", "TEAM=t\nAGENT=member\n", "TEAM=t\nAGENT=reviewer\n",
		}},
		{name: "exact membership and missing names", agent: "lead", team: "t", records: []string{
			"TEAM=other\nAGENT=member\n", "TEAM=tt\nAGENT=member\n", "TEAM=t\nAGENT=\n", "TEAM=t\n", "AGENT=member\n",
		}},
		{name: "unknown and duplicate members retain order", agent: "lead", team: "t", records: []string{
			"TEAM=t\nAGENT=zeta\n", "TEAM=t\nAGENT=member\n", "TEAM=t\nAGENT=zeta\n",
		}},
		{name: "metadata is raw text", agent: "lead", team: "t", records: []string{
			"TEAM='t'\nAGENT=member\n", "TEAM=t\nAGENT='member'\n", "TEAM=t\nAGENT=unconfigured",
		}},
		{name: "duplicate assignments follow grep and sed", agent: "lead", team: "t", records: []string{
			"TEAM=t\nTEAM=other\nAGENT=member\nAGENT=reviewer\n", "TEAM=t\nAGENT=member\nAGENT=\n",
		}},
		{name: "literal indent and identity", indent: " > ", agent: "unknown-name", team: "t.with[punctuation]", records: []string{
			"TEAM=t.with[punctuation]\nAGENT=member\n", "TEAM=tXwithp\nAGENT=member\n",
		}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var metas []MetaFile
			for _, record := range tt.records {
				metas = append(metas, parseMeta([]byte(record)))
			}
			want := attributionBashOracle(t, tt.indent, tt.agent, tt.team, tt.records)
			got := FormatAttributionBlock(tt.indent, tt.agent, tt.team, metas, config)
			if got != want {
				t.Fatalf("footer = %q, Bash = %q", got, want)
			}
		})
	}
	want := "Opened by team-agents:\n- agent: `lead` (codex, model `model-a`)\n"
	if got := FormatAttributionBlock("", "lead", "", nil, config); got != want {
		t.Fatalf("standalone footer = %q, want %q", got, want)
	}
}

// attributionBashOracle extracts only frozen pure helpers, avoiding launcher startup.
func attributionBashOracle(t *testing.T, indent, agent, team string, records []string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "bin", "agent"))
	if err != nil {
		t.Fatal(err)
	}
	source := string(data)
	var functions strings.Builder
	for _, bounds := range [][2]string{
		{"agent_desc() {", "\n# agent_github_colors"},
		{"attribution_block() {", "\nround_check() {"},
	} {
		start := strings.Index(source, bounds[0])
		if start < 0 {
			t.Fatalf("missing Bash helper %q", bounds[0])
		}
		end := strings.Index(source[start:], bounds[1])
		if end < 0 {
			t.Fatalf("missing Bash helper boundary %q", bounds[1])
		}
		functions.WriteString(source[start : start+end])
		functions.WriteByte('\n')
	}
	tasks := t.TempDir()
	for i, record := range records {
		dir := filepath.Join(tasks, fmt.Sprintf("%02d", i))
		if err := os.Mkdir(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "meta.env"), []byte(record), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	script := `declare -A AGENT_TYPE=([lead]=codex [member]=claude)
declare -A AGENT_MODEL=([lead]=model-a [member]=model-b)
declare -A INVESTIGATORS_TYPE=([reviewer]=codex)
declare -A INVESTIGATORS_MODEL=([reviewer]=review-model)
` + functions.String() + "\nAGENT=$1 TEAM=$2 TASKS=$3\nattribution_block \"$4\"\n"
	return runMetaBash(t, "C.UTF-8", script, agent, team, tasks, indent)
}
