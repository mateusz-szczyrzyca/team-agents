package main

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestIdentityFromRunArrayConfigLoader verifies that array-form declarations in agents.conf
// are correctly loaded via the config loader and honored during identity derivation and setup.
func TestIdentityFromRunArrayConfigLoader(t *testing.T) {
	home := t.TempDir()
	keyPath := filepath.Join(home, "id_test")
	if err := os.WriteFile(keyPath, []byte("key"), 0o600); err != nil {
		t.Fatal(err)
	}
	confPath := filepath.Join(home, "agents.conf")
	conf := "GIT_NAME_FORMAT=('%a custom')\nGIT_EMAIL=('%a@example.com')\nGIT_SIGNING_KEY=('" + keyPath + "')\n"
	if err := os.WriteFile(confPath, []byte(conf), 0o600); err != nil {
		t.Fatal(err)
	}
	req := ConfigRequest{File: confPath, Context: ConfigContext{Home: home, AgentsHome: home}}
	cfg, loadErr := newApp(OSExecutor{}, nil, nil).loadConfig(context.Background(), req)
	if loadErr != nil {
		t.Fatalf("loadConfig failed: %v", loadErr)
	}
	meta := MetaFile{Lines: []MetaLine{{Kind: MetaAssignment, Key: "AGENT", Value: "probe"}}}
	id := IdentityFromRun(meta, cfg, "run-1")
	env, setupErr := id.Setup(context.Background(), "", nil)
	if setupErr != nil {
		t.Fatalf("id.Setup failed: %v", setupErr)
	}
	if env["GIT_AUTHOR_NAME"] != "probe custom" || env["GIT_AUTHOR_EMAIL"] != "probe@example.com" || env["GIT_CONFIG_COUNT"] != "4" {
		t.Fatalf("unexpected env: author=%q email=%q signing_count=%q", env["GIT_AUTHOR_NAME"], env["GIT_AUTHOR_EMAIL"], env["GIT_CONFIG_COUNT"])
	}
}
