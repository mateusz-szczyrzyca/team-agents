package main

import (
	"context"
	"path/filepath"
	"testing"
)

// TestConfigReviewValues checks Bash values directly, independently of snapshot serialization.
func TestConfigReviewValues(t *testing.T) {
	ctx := ConfigContext{Home: "/tmp/h", AgentsHome: "/tmp/h/agents", Hostname: "node.example", UID: "1234"}
	for _, tc := range []struct {
		script, name, key, want string
		invalid                 bool
	}{
		{`pool=(first second); declare -n NORMAL_AGENTS=pool`, "NORMAL_AGENTS", "1", "second", false},
		{`declare -u AGENT_LANGUAGE=''`, "AGENT_LANGUAGE", "", "ENGLISH", false},
		{`set --; AGENT_LANGUAGE=Polish`, "AGENT_LANGUAGE", "", "Polish", false},
		{`v=x; AGENT_LANGUAGE=${v//x/&}`, "AGENT_LANGUAGE", "", "&", false},
		{`declare -n NOTIFY_DIR=ENV_NOTES_FILE`, "ENV_NOTES_FILE", "", "/tmp/h/agents/notifications", false},
		{`declare -a INVESTIGATORS_TYPE=() INVESTIGATORS_MODEL=() INVESTIGATORS_EXTRA_ARGS=(); INVESTIGATOR_NAME=02`, "INVESTIGATORS_TYPE", "2", "claude", false},
		{`declare -a INVESTIGATORS_TYPE=() INVESTIGATORS_MODEL=() INVESTIGATORS_EXTRA_ARGS=(); INVESTIGATOR_NAME=2+3`, "INVESTIGATORS_TYPE", "5", "claude", false},
		{`INVESTIGATOR_NAME=2+3`, "INVESTIGATORS_TYPE", "2+3", "claude", false},
		{`declare -a INVESTIGATORS_TYPE=() INVESTIGATORS_MODEL=() INVESTIGATORS_EXTRA_ARGS=(); INVESTIGATOR_NAME=1+`, "", "", "", true},
	} {
		t.Run(tc.script, func(t *testing.T) {
			file := filepath.Join(t.TempDir(), "config")
			writeContextFixture(t, file, tc.script+"\n")
			app := newApp(OSExecutor{}, nil, nil)
			probe := `[[ ${` + tc.name + `} == '` + tc.want + `' ]]`
			if tc.key != "" {
				probe = `[[ ${` + tc.name + `[` + tc.key + `]} == '` + tc.want + `' ]]`
			}
			// Bash before 5.2 lacks this option; match the production startup guard.
			oracle := `set -euo pipefail; shopt -u patsub_replacement 2>/dev/null || :; CONF=$1 HOME=$2 AGENTS_HOME=$3 HOSTNAME=$4 SELF=$5; TASKS=$AGENTS_HOME/tasks PROMPTS=$AGENTS_HOME/prompts BUS_DIR=$AGENTS_HOME/bus LOG_DIR=$AGENTS_HOME/logs; eval "$(sed -n '/source "\$CONF"/,/export XDG_RUNTIME_DIR/p' ../bin/agent)"; ` + probe
			result, failure := app.execute(context.Background(), Command{Name: "bash", Args: []string{"-c", oracle, "oracle", file, ctx.Home, ctx.AgentsHome, ctx.Hostname, ctx.Self}, Env: []string{"PATH=/usr/bin:/bin"}})
			if failure != nil || (result.ExitCode != 0) != tc.invalid {
				t.Fatalf("direct Bash assertion: exit=%d error=%v", result.ExitCode, failure)
			}
			cfg, loadErr := app.loadConfig(context.Background(), ConfigRequest{File: file, Env: []string{"PATH=/usr/bin:/bin"}, Context: ctx})
			if tc.invalid {
				if loadErr == nil || loadErr.Cause != LoadConfigShell {
					t.Fatalf("invalid Bash subscript: %v", loadErr)
				}
				return
			}
			got, _ := configScalar(cfg, tc.name)
			if tc.key != "" {
				got = cfg.Indexed[tc.name][tc.key]
				if values, ok := cfg.Associative[tc.name]; ok {
					got = values[tc.key]
				}
			}
			if loadErr != nil || got != tc.want {
				t.Fatalf("want %q, got %q, error=%v", tc.want, got, loadErr)
			}
		})
	}
}

// TestConfigReviewExtraArgs loads the existing fixture under production shell options.
func TestConfigReviewExtraArgs(t *testing.T) {
	app := newApp(OSExecutor{}, nil, nil)
	cfg, err := app.loadConfig(context.Background(), ConfigRequest{
		File: "testdata/config-extra-args.conf", Env: []string{"PATH=/usr/bin:/bin"},
		Context: ConfigContext{Home: t.TempDir(), Hostname: "test", UID: "1234"},
	})
	if err != nil || cfg.Associative["AGENT_EXTRA_ARGS"]["split-test"] == "" {
		t.Fatalf("extra-args fixture: %v", err)
	}
}
