package main

import (
	"context"
	"path/filepath"
	"testing"
)

// TestLoadConfigAcceptsModelPrices: agents.conf may price models in MODEL_PRICES, an associative array the Bash
// program declares before sourcing it (agent spend). The native loader declares it too: a priced model is no error.
func TestLoadConfigAcceptsModelPrices(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("AGENTS_HOME", home)
	writeConfigFixture(t, filepath.Join(home, "agents.conf"), `MODEL_PRICES[gpt-6.1-sol]="2 0.10 10"
MODEL_PRICES[deepinfra/Qwen/Qwen3-Coder-480B-A35B-Instruct]="0.40 0.40 1.60"
`)
	app := newApp(OSExecutor{}, nil, nil)
	if err := app.loadStartupConfig(context.Background(), nil); err != nil {
		t.Fatalf("loadStartupConfig with MODEL_PRICES failed: %v", err)
	}
}
