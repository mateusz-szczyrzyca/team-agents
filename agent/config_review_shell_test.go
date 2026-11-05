package main

import (
	"context"
	"os"
	"testing"
)

// TestReviewAllowlistWithSetDashAndOptions verifies allowlist survives set -- and pre-source options.
func TestReviewAllowlistWithSetDashAndOptions(t *testing.T) {
	app := newApp(OSExecutor{}, nil, nil)
	for _, tc := range []struct {
		name   string
		config string
		check  string
		want   string
	}{
		{"M3: set -- preserves allowlist", "set --\nAGENT_LANGUAGE=Polish\n", "AGENT_LANGUAGE", "Polish"},
		{"inv-kimi: pre-source shopt -u", "shopt -u patsub_replacement\nAGENT_LANGUAGE=Arabic\n", "AGENT_LANGUAGE", "Arabic"},
		{"m1: associative extra-args", "declare -A AGENT_EXTRA_ARGS\nAGENT_EXTRA_ARGS[split-test]='--verbose'\n", "extra", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			tempDir := t.TempDir()
			confFile := tempDir + "/config.conf"
			if err := os.WriteFile(confFile, []byte(tc.config), 0600); err != nil {
				t.Fatalf("WriteFile: %v", err)
			}
			req := ConfigRequest{
				File: confFile,
				Context: ConfigContext{
					Home:       tempDir,
					AgentsHome: tempDir,
					Hostname:   "test",
					Self:       "self",
				},
				Env: []string{"HOME=" + tempDir},
			}
			cfg, err := app.loadShellConfig(context.Background(), req)
			if err != nil {
				t.Fatalf("loadShellConfig: %v", err)
			}
			if tc.check == "extra" {
				assocMap, ok := cfg.Associative["AGENT_EXTRA_ARGS"]
				if !ok || assocMap["split-test"] != "--verbose" {
					t.Errorf("AGENT_EXTRA_ARGS[split-test] failed")
				}
			} else {
				val, ok := configScalar(cfg, tc.check)
				if !ok || val != tc.want {
					t.Errorf("got %q (ok=%v), want %q (ok=true)", val, ok, tc.want)
				}
			}
		})
	}
}

// TestExtraArgsFixtureUnderProduction verifies config-extra-args fixture works with production options.
func TestExtraArgsFixtureUnderProduction(t *testing.T) {
	tempDir := t.TempDir()
	confFile := tempDir + "/config.conf"
	// Fixture with m1 fix: declare -A prepended to load under bash -euc.
	if err := os.WriteFile(confFile, []byte(
		"declare -A AGENT_EXTRA_ARGS\nAGENT_EXTRA_ARGS[split-test]=\"--flag1 'quoted arg'\n--flag2\"\n"+
			"RUN_PATH=\"/custom/path/bin:/usr/bin:/bin\"\n"), 0600); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}
	cfg, _ := newApp(OSExecutor{}, nil, nil).loadShellConfig(context.Background(), ConfigRequest{
		File: confFile,
		Context: ConfigContext{Home: tempDir, AgentsHome: tempDir, Hostname: "test", Self: "self"},
		Env:     []string{"HOME=" + tempDir},
	})
	m := cfg.Associative["AGENT_EXTRA_ARGS"]
	if m == nil || m["split-test"] != "--flag1 'quoted arg'\n--flag2" {
		t.Error("AGENT_EXTRA_ARGS[split-test] mismatch")
	}
}
