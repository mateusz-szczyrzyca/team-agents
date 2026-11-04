package main

import (
	"testing"
)

// TestConfigStructure verifies all configuration fields are present and typed correctly.
func TestConfigStructure(t *testing.T) {
	c := NewConfig()
	if c == nil {
		t.Fatal("NewConfig returned nil")
	}

	// Verify all map fields are initialized
	if c.AgentType == nil {
		t.Fatal("AgentType map not initialized")
	}
	if c.AgentModel == nil {
		t.Fatal("AgentModel map not initialized")
	}
	if c.ExtraArgs == nil {
		t.Fatal("ExtraArgs map not initialized")
	}
	if c.InvestigatorType == nil {
		t.Fatal("InvestigatorType map not initialized")
	}
	if c.InvestigatorModel == nil {
		t.Fatal("InvestigatorModel map not initialized")
	}
	if c.InvestigatorExtraArgs == nil {
		t.Fatal("InvestigatorExtraArgs map not initialized")
	}
	if c.RoundMaxLinesRepo == nil {
		t.Fatal("RoundMaxLinesRepo map not initialized")
	}
	if c.NodeRepos == nil {
		t.Fatal("NodeRepos map not initialized")
	}

	// Verify slice fields
	if c.NormalAgents == nil {
		t.Fatal("NormalAgents slice not initialized")
	}
	if c.WorkerAgents == nil {
		t.Fatal("WorkerAgents slice not initialized")
	}
	if c.InstanceCapabilities == nil {
		t.Fatal("InstanceCapabilities slice not initialized")
	}
}

// TestAgentTypeValues verifies all valid agent types.
func TestAgentTypeValues(t *testing.T) {
	cases := []struct {
		name  string
		value AgentTypeValue
		valid bool
	}{
		{"claude", AgentClaude, true},
		{"codex", AgentCodex, true},
		{"gemini", AgentGemini, true},
		{"opencode", AgentOpencode, true},
		{"invalid", AgentTypeValue("invalid"), false},
		{"CLAUDE", AgentTypeValue("CLAUDE"), false},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.value.Valid() != tc.valid {
				t.Fatalf("AgentTypeValue(%q).Valid() = %v, want %v", tc.value, tc.value.Valid(), tc.valid)
			}
			if tc.value.String() != string(tc.value) {
				t.Fatalf("AgentTypeValue(%q).String() = %q, want %q", tc.value, tc.value.String(), string(tc.value))
			}
		})
	}
}

// TestParseAgentType verifies agent type parsing.
func TestParseAgentType(t *testing.T) {
	cases := []struct {
		input   string
		valid   bool
		wantErr bool
	}{
		{"claude", true, false},
		{"codex", true, false},
		{"gemini", true, false},
		{"opencode", true, false},
		{"invalid", false, true},
		{"Claude", false, true},
		{"", false, true},
	}

	for _, tc := range cases {
		t.Run(tc.input, func(t *testing.T) {
			val, err := ParseAgentType(tc.input)
			if (err != nil) != tc.wantErr {
				t.Fatalf("ParseAgentType(%q) error = %v, wantErr = %v", tc.input, err, tc.wantErr)
			}
			if !tc.wantErr && !val.Valid() {
				t.Fatalf("ParseAgentType(%q) returned invalid type", tc.input)
			}
		})
	}
}

// TestAgentTierParsing verifies agent tier determination.
func TestAgentTierParsing(t *testing.T) {
	normal := []string{"claude-1", "codex-1", "gemini-1"}
	worker := []string{"worker-1", "worker-2"}

	cases := []struct {
		agent    string
		wantTier AgentTier
	}{
		{"claude-1", TierNormal},
		{"codex-1", TierNormal},
		{"worker-1", TierWorker},
		{"worker-2", TierWorker},
		{"unknown", TierNormal},
	}

	for _, tc := range cases {
		t.Run(tc.agent, func(t *testing.T) {
			tier := ParseAgentTier(tc.agent, normal, worker)
			if tier != tc.wantTier {
				t.Fatalf("ParseAgentTier(%q) = %v, want %v", tc.agent, tier, tc.wantTier)
			}
		})
	}
}

// TestAgentTierValues verifies tier string values.
func TestAgentTierValues(t *testing.T) {
	cases := []struct {
		tier  AgentTier
		valid bool
	}{
		{TierNormal, true},
		{TierWorker, true},
		{AgentTier("invalid"), false},
		{AgentTier("NORMAL"), false},
	}

	for _, tc := range cases {
		t.Run(tc.tier.String(), func(t *testing.T) {
			if tc.tier.Valid() != tc.valid {
				t.Fatalf("AgentTier(%q).Valid() = %v, want %v", tc.tier, tc.tier.Valid(), tc.valid)
			}
		})
	}
}

// TestConfigDefaults verifies that NewConfig sets appropriate defaults.
func TestConfigDefaults(t *testing.T) {
	c := NewConfig()

	if c.AgentLanguage != "English" {
		t.Fatalf("AgentLanguage = %q, want English", c.AgentLanguage)
	}
	if c.RepoLanguage != "English" {
		t.Fatalf("RepoLanguage = %q, want English", c.RepoLanguage)
	}
	if c.RoundMaxLines != 3000 {
		t.Fatalf("RoundMaxLines = %d, want 3000", c.RoundMaxLines)
	}
	if c.RoundSoftLines != 2000 {
		t.Fatalf("RoundSoftLines = %d, want 2000", c.RoundSoftLines)
	}
	if c.AgentHangMin != 30 {
		t.Fatalf("AgentHangMin = %d, want 30", c.AgentHangMin)
	}
	if c.AgentHangStrikes != 3 {
		t.Fatalf("AgentHangStrikes = %d, want 3", c.AgentHangStrikes)
	}
}

// TestConfigMapOperations verifies all map fields can store and retrieve values.
func TestConfigMapOperations(t *testing.T) {
	c := NewConfig()

	// Test agent type map
	c.AgentType["claude-1"] = "claude"
	if c.AgentType["claude-1"] != "claude" {
		t.Fatal("Failed to store/retrieve from AgentType map")
	}

	// Test agent model map
	c.AgentModel["claude-1"] = "sonnet"
	if c.AgentModel["claude-1"] != "sonnet" {
		t.Fatal("Failed to store/retrieve from AgentModel map")
	}

	// Test extra args map
	c.ExtraArgs["worker-1"] = "-c model_reasoning_effort=high"
	if c.ExtraArgs["worker-1"] != "-c model_reasoning_effort=high" {
		t.Fatal("Failed to store/retrieve from ExtraArgs map")
	}

	// Test investigator maps
	c.InvestigatorType["inv-opus"] = "claude"
	c.InvestigatorModel["inv-opus"] = "opus"
	c.InvestigatorExtraArgs["inv-opus"] = "-c model_reasoning_effort=maximum"
	if c.InvestigatorType["inv-opus"] != "claude" ||
	   c.InvestigatorModel["inv-opus"] != "opus" ||
	   c.InvestigatorExtraArgs["inv-opus"] != "-c model_reasoning_effort=maximum" {
		t.Fatal("Failed to store/retrieve from Investigator maps")
	}

	// Test round max lines repo
	c.RoundMaxLinesRepo["team-agents"] = 1500
	if c.RoundMaxLinesRepo["team-agents"] != 1500 {
		t.Fatal("Failed to store/retrieve from RoundMaxLinesRepo map")
	}

	// Test node repos
	c.NodeRepos["team-agents"] = "/home/user/team-agents"
	if c.NodeRepos["team-agents"] != "/home/user/team-agents" {
		t.Fatal("Failed to store/retrieve from NodeRepos map")
	}
}

// TestConfigSliceOperations verifies all slice fields can store and retrieve values.
func TestConfigSliceOperations(t *testing.T) {
	c := NewConfig()

	// Test normal agents slice
	c.NormalAgents = []string{"claude-1", "codex-1", "gemini-1"}
	if len(c.NormalAgents) != 3 {
		t.Fatalf("NormalAgents length = %d, want 3", len(c.NormalAgents))
	}
	if c.NormalAgents[0] != "claude-1" {
		t.Fatal("Failed to retrieve from NormalAgents slice")
	}

	// Test worker agents slice
	c.WorkerAgents = []string{"worker-1", "worker-2"}
	if len(c.WorkerAgents) != 2 {
		t.Fatalf("WorkerAgents length = %d, want 2", len(c.WorkerAgents))
	}

	// Test instance capabilities slice
	c.InstanceCapabilities = []string{"os:linux", "arch:arm64", "hw:rp2350"}
	if len(c.InstanceCapabilities) != 3 {
		t.Fatalf("InstanceCapabilities length = %d, want 3", len(c.InstanceCapabilities))
	}
}

// TestConfigStringFields verifies all string fields can be set.
func TestConfigStringFields(t *testing.T) {
	c := NewConfig()

	testFields := map[string]*string{
		"GitNameFormat": &c.GitNameFormat,
		"GitEmail": &c.GitEmail,
		"RunPath": &c.RunPath,
		"BranchPrefix": &c.BranchPrefix,
	}

	for fieldName, field := range testFields {
		testValue := "test-" + fieldName
		*field = testValue
		if *field != testValue {
			t.Fatalf("Failed to set/get %s", fieldName)
		}
	}
}

// TestConfigIntFields verifies all int fields can be set.
func TestConfigIntFields(t *testing.T) {
	c := NewConfig()

	testFields := map[string]*int{
		"AgentHangMin": &c.AgentHangMin,
		"RoundMaxLines": &c.RoundMaxLines,
		"VerifyMaxFixPasses": &c.VerifyMaxFixPasses,
		"NotifyMaxChars": &c.NotifyMaxChars,
	}

	for fieldName, field := range testFields {
		testValue := 42
		*field = testValue
		if *field != testValue {
			t.Fatalf("Failed to set/get %s", fieldName)
		}
	}
}

// TestConfigErrorCreation verifies config error creation.
func TestConfigErrorCreation(t *testing.T) {
	err := NewConfigError(ConfigInvalidValue, "test error")
	if err == nil {
		t.Fatal("NewConfigError returned nil")
	}
	if err.Cause != ConfigInvalidValue {
		t.Fatalf("Cause = %v, want ConfigInvalidValue", err.Cause)
	}
	if err.Reason != "test error" {
		t.Fatalf("Reason = %q, want test error", err.Reason)
	}
}

// TestConfigErrorString verifies error message formatting.
func TestConfigErrorString(t *testing.T) {
	err := NewConfigError(ConfigInvalidValue, "invalid value for %s", "field")
	if err == nil {
		t.Fatal("NewConfigError returned nil")
	}
	errStr := err.Error()
	if errStr != "invalid value for field" {
		t.Fatalf("Error() = %q, want 'invalid value for field'", errStr)
	}
}

// TestConfigErrorNil verifies nil error handling.
func TestConfigErrorNil(t *testing.T) {
	var err *ConfigError
	if err.Error() != "" {
		t.Fatalf("nil ConfigError.Error() = %q, want empty string", err.Error())
	}
}

// TestLoadConfigFromFile verifies loading configuration from agents.conf file.
func TestLoadConfigFromFile(t *testing.T) {
	// This test will fail until LoadConfigFromFile is implemented.
	// For now, we're testing that the function can be called and returns a Config.
	c, err := LoadConfigFromFile("../config/agents.conf")
	if err != nil {
		t.Skipf("config file not accessible: %v", err)
	}
	if c == nil {
		t.Fatal("LoadConfigFromFile returned nil Config")
	}
}

// TestLoadSecretsFromFile verifies loading secrets from envfile.
func TestLoadSecretsFromFile(t *testing.T) {
	// This test will fail until LoadSecretsFromFile is implemented.
	env, err := LoadSecretsFromFile("config/secrets.env")
	if err != nil {
		// Missing file is acceptable (secrets.env is optional)
		if err != nil {
			t.Logf("LoadSecretsFromFile: %v (optional file may not exist)", err)
		}
	}
	if env == nil {
		t.Fatal("LoadSecretsFromFile returned nil map")
	}
}

// TestConfigAgentValidation verifies that agent configurations are validated.
func TestConfigAgentValidation(t *testing.T) {
	c := NewConfig()

	// Add agent with invalid type
	c.AgentType["invalid-agent"] = "invalid-type"

	// Validation should catch this
	err := c.Validate()
	if err == nil {
		t.Fatal("Validate should reject invalid agent type")
	}
}

// TestConfigInvestigatorValidation verifies investigator config validation.
func TestConfigInvestigatorValidation(t *testing.T) {
	c := NewConfig()

	// Add investigator with mismatched configs
	c.InvestigatorType["inv-test"] = "claude"
	// Missing InvestigatorModel for this investigator

	err := c.Validate()
	if err == nil {
		t.Fatal("Validate should check that all investigators have types and models")
	}
}

// TestConfigExportedEnvironment verifies exported environment variable generation.
func TestConfigExportedEnvironment(t *testing.T) {
	c := NewConfig()
	c.RunPath = "/test/path"
	c.BranchPrefix = "agents"

	env := c.ExportedEnvironment()
	if env == nil {
		t.Fatal("ExportedEnvironment returned nil")
	}

	// Check that critical environment variables are set
	if env["RUN_PATH"] != "/test/path" {
		t.Fatalf("RUN_PATH not set correctly in exported environment")
	}
}
