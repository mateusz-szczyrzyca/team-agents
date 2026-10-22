package main

import (
	"encoding/json"
	"testing"
	"time"
)

// TestCapabilityValidation tests the closed vocabulary validation.
func TestCapabilityValidation(t *testing.T) {
	valid := []Capability{
		"os:linux", "os:darwin", "build:ios", "build:rust", "sign:apple", "publish:npm",
	}
	for _, c := range valid {
		if !c.Valid() {
			t.Errorf("expected capability %q to be valid", c)
		}
	}

	invalid := []string{
		"unknown", "build:unknown", "os:windows", "sign:unknown", "",
	}
	for _, s := range invalid {
		if Capability(s).Valid() {
			t.Errorf("expected capability %q to be invalid", s)
		}
		if _, err := ParseCapability(s); err == nil {
			t.Errorf("expected ParseCapability(%q) to fail", s)
		}
	}
}

// TestCapabilityNamespaces tests namespace extraction and sensitivity classification.
func TestCapabilityNamespaces(t *testing.T) {
	if got := Capability("build:ios").Namespace(); got != "build:" {
		t.Errorf("expected build:, got %q", got)
	}
	if got := Capability("invalid").Namespace(); got != "" {
		t.Errorf("expected empty namespace for invalid format, got %q", got)
	}

	if !Capability("sign:apple").IsSensitive() {
		t.Error("expected sign:apple to be sensitive")
	}
	if !Capability("publish:npm").IsSensitive() {
		t.Error("expected publish:npm to be sensitive")
	}
	if Capability("build:ios").IsSensitive() {
		t.Error("expected build:ios not to be sensitive")
	}
}

// TestInstanceManifestJSON tests JSON encoding and decoding of an InstanceManifest.
func TestInstanceManifestJSON(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Second)
	manifest := InstanceManifest{
		InstanceName: "mac",
		Timestamp:    now,
		TTLSeconds:   180,
		Detected: DetectedCapabilities{
			OS:         "darwin",
			Arch:       "arm64",
			Toolchains: []string{"go1.23", "rust1.82"},
		},
		Declared: []Capability{"build:ios", "sign:apple"},
		CurrentState: InstanceCurrentState{
			Slots: SlotCapacity{Total: 4, Free: 3},
			Agents: []ManifestAgentState{
				{
					Name:   "claude-1",
					Tier:   TierNormal,
					Role:   "backend",
					Status: "idle",
				},
			},
			Subscriptions: map[string]SubscriptionQuota{
				"anthropic": {Exhausted: false},
			},
			Tasks: map[string]HubTaskSummary{
				"task-1": {
					TaskID:    "task-1",
					Agent:     "claude-1",
					Team:      "team-1",
					Status:    "running",
					StartedAt: now,
					UpdatedAt: now,
				},
			},
		},
	}

	data, err := json.Marshal(manifest)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}

	var decoded InstanceManifest
	if err := json.Unmarshal(data, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}

	if decoded.InstanceName != "mac" || decoded.TTLSeconds != 180 {
		t.Errorf("unexpected decoded values: %+v", decoded)
	}
	if len(decoded.Declared) != 2 || decoded.Declared[0] != "build:ios" {
		t.Errorf("declared capabilities mismatch: %+v", decoded.Declared)
	}
	if decoded.CurrentState.Slots.Total != 4 || decoded.CurrentState.Slots.Free != 3 {
		t.Errorf("slots mismatch: %+v", decoded.CurrentState.Slots)
	}
}
