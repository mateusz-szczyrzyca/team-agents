package main

import (
	"fmt"
	"os"
)

// Config holds all agent runtime configuration loaded from agents.conf and secrets.env.
type Config struct {
	// Agent configuration
	AgentType   map[string]string // agent name → CLI type (claude|codex|gemini|opencode)
	AgentModel  map[string]string // agent name → model name or preset
	ExtraArgs   map[string]string // agent name → extra CLI arguments

	// Agent tiers
	NormalAgents []string // normal model agents that can coordinate
	WorkerAgents []string // worker agents for mechanical tasks

	// Investigators
	InvestigatorType   map[string]string // investigator name → CLI type
	InvestigatorModel  map[string]string // investigator name → model
	InvestigatorExtraArgs map[string]string // investigator name → extra CLI arguments
	InvestigationQueueMax int // max waiting requests in investigation queue

	// Watchdog and wake-loop guards
	AgentHangMin           int
	AgentHangStrikes       int
	AgentIdleRunSec        int
	AgentWakeLoopMax       int
	AgentWakeRateMax       int
	AgentWakeLoopPauseMin  int

	// Quota and rate limiting
	AgentQuotaPauseMin      int
	AgentReleasePauseMin    int
	AgentRateLimitPauseMin  int
	AgentLimitTailLines     int

	// Language and environment
	AgentLanguage string
	RepoLanguage  string

	// Commit identity
	GitNameFormat    string
	GitEmail         string
	GitSigningKey    string
	EnvNotesFile     string

	// Paths
	ClaudeBin       string
	CodexBin        string
	AgyBin          string
	OpencodeBin     string
	OpencodeBashTimeoutMs int
	AgentbusBin     string
	AgentSerena     string
	RunPath         string

	// Notifications
	NotifyCmd          string
	QuestionsEditor    string
	VscodeServerDirs   string
	AgentConsultMin    int

	// Review and verification
	VerifyMaxFixPasses   int
	AgentHumanGithub     string
	AgentReviewDraw      string
	AgentReviewRotateWaitMin int

	// GitHub colors
	AgentGithubColors string

	// Rounds
	RoundMaxLines     int
	RoundSoftLines    int
	RoundExclude      string
	RoundBounces      int
	RoundMaxLinesRepo map[string]int // repo name → max lines

	// Node jobs
	NodeRepos map[string]string // repo name → local path

	// Web interface
	AgentWebListen  string
	AgentWebUsers   string
	AgentWebDevices string
	AgentWebUrl     string

	// Lead and node settings
	LeadListen           string
	LeadNodes            string
	InstanceName         string
	NodeTtlSec           int
	LeadUrl              string
	NodePollSec          int
	InstanceCapabilities []string
	NotifyDir            string
	NotifyMaxChars       int

	// Agent live
	LiveRemindMin   int
	AgentliveBin    string

	// Auth models for testing
	AuthModelAnthropic   string
	AuthModelOpenai      string
	AuthModelGemini      string
	AuthModelOpenrouter  string
	AuthModelDeepinfra   string

	// Misc
	BranchPrefix    string
	ClaudeMaxTurns  int
	AgyPrintTimeout string
}

// AgentTypeValue represents a valid agent CLI type.
type AgentTypeValue string

const (
	AgentClaude   AgentTypeValue = "claude"
	AgentCodex    AgentTypeValue = "codex"
	AgentGemini   AgentTypeValue = "gemini"
	AgentOpencode AgentTypeValue = "opencode"
)

// Valid checks if the agent type is recognized.
func (a AgentTypeValue) Valid() bool {
	switch a {
	case AgentClaude, AgentCodex, AgentGemini, AgentOpencode:
		return true
	}
	return false
}

// String returns the string representation.
func (a AgentTypeValue) String() string {
	return string(a)
}

// ParseAgentType parses a string into an AgentTypeValue.
func ParseAgentType(s string) (AgentTypeValue, error) {
	val := AgentTypeValue(s)
	if !val.Valid() {
		return "", NewConfigError(ConfigInvalidValue, "invalid agent type: %s", s)
	}
	return val, nil
}

// AgentTier represents whether an agent is normal or worker.
type AgentTier string

const (
	TierNormal AgentTier = "normal"
	TierWorker AgentTier = "worker"
)

// Valid checks if the tier is recognized.
func (t AgentTier) Valid() bool {
	switch t {
	case TierNormal, TierWorker:
		return true
	}
	return false
}

// String returns the string representation.
func (t AgentTier) String() string {
	return string(t)
}

// ParseAgentTier determines if an agent is normal or worker based on config lists.
func ParseAgentTier(agentName string, normalAgents, workerAgents []string) AgentTier {
	for _, a := range workerAgents {
		if a == agentName {
			return TierWorker
		}
	}
	return TierNormal
}

// NewConfig creates a Config with sensible defaults from agents.conf.
func NewConfig() *Config {
	return &Config{
		AgentType:   make(map[string]string),
		AgentModel:  make(map[string]string),
		ExtraArgs:   make(map[string]string),
		InvestigatorType:   make(map[string]string),
		InvestigatorModel:  make(map[string]string),
		InvestigatorExtraArgs: make(map[string]string),
		RoundMaxLinesRepo: make(map[string]int),
		NodeRepos:         make(map[string]string),
		NormalAgents: []string{},
		WorkerAgents: []string{},
		InstanceCapabilities: []string{},
		InvestigationQueueMax: 3,
		AgentLanguage:     "English",
		RepoLanguage:      "English",
		AgentHangMin:      30,
		AgentHangStrikes:  3,
		AgentIdleRunSec:   120,
		AgentWakeLoopMax:  4,
		AgentWakeRateMax:  30,
		AgentWakeLoopPauseMin:  15,
		AgentQuotaPauseMin:     30,
		AgentReleasePauseMin:   20,
		AgentRateLimitPauseMin: 5,
		AgentLimitTailLines:    30,
		GitNameFormat:     "%a (team-agents)",
		RoundMaxLines:     3000,
		RoundSoftLines:    2000,
		RoundExclude:      "*_gen.go go.sum package-lock.json vendor/",
		RoundBounces:      2,
		BranchPrefix:      "agents",
		ClaudeMaxTurns:    300,
		AgyPrintTimeout:   "8h",
		LiveRemindMin:     1,
		NodePollSec:       5,
		NodeTtlSec:        90,
		NotifyMaxChars:    1500,
		VerifyMaxFixPasses: 2,
		AgentReviewDraw:   "random",
		AgentReviewRotateWaitMin: 20,
	}
}

// ConfigErrorCause represents a configuration error type.
type ConfigErrorCause int

const (
	ConfigInvalidValue ConfigErrorCause = iota
	ConfigMissingValue
	ConfigTypeMismatch
)

// ConfigError represents a configuration loading error.
type ConfigError struct {
	Cause  ConfigErrorCause
	File   string
	Line   int
	Reason string
}

// Error implements the error interface.
func (e *ConfigError) Error() string {
	if e == nil {
		return ""
	}
	return e.Reason
}

// NewConfigError creates a new configuration error.
func NewConfigError(cause ConfigErrorCause, format string, args ...interface{}) *ConfigError {
	return &ConfigError{
		Cause:  cause,
		Reason: fmt.Sprintf(format, args...),
	}
}

// LoadConfigFromFile loads configuration from agents.conf file via Bash sourcing.
func LoadConfigFromFile(filePath string) (*Config, error) {
	c := NewConfig()

	if _, err := os.Stat(filePath); err != nil {
		return nil, fmt.Errorf("failed to load config from %s: %v", filePath, err)
	}

	// Stub: actual Bash sourcing will be implemented in config_shell.go
	return c, nil
}

// LoadSecretsFromFile loads secrets from envfile.
func LoadSecretsFromFile(filePath string) (map[string]string, error) {
	env := make(map[string]string)

	// Stub: file is optional, missing is not an error
	// Actual implementation will be in envfile.go

	return env, nil
}

// Validate checks that all required fields are set and internally consistent.
func (c *Config) Validate() error {
	if len(c.AgentType) == 0 {
		return NewConfigError(ConfigMissingValue, "no agents configured")
	}

	// Validate agent types
	for name, typeStr := range c.AgentType {
		if _, err := ParseAgentType(typeStr); err != nil {
			return fmt.Errorf("agent %q has invalid type %q: %v", name, typeStr, err)
		}
	}

	// Validate investigator consistency
	for name := range c.InvestigatorType {
		if _, ok := c.InvestigatorModel[name]; !ok {
			return fmt.Errorf("investigator %q has type but no model", name)
		}
	}
	for name := range c.InvestigatorModel {
		if _, ok := c.InvestigatorType[name]; !ok {
			return fmt.Errorf("investigator %q has model but no type", name)
		}
	}

	return nil
}

// ExportedEnvironment returns environment variables that should be set for agent runs.
func (c *Config) ExportedEnvironment() map[string]string {
	env := make(map[string]string)

	if c.RunPath != "" {
		env["RUN_PATH"] = c.RunPath
	}
	if c.BranchPrefix != "" {
		env["BRANCH_PREFIX"] = c.BranchPrefix
	}
	if c.AgentLanguage != "" {
		env["AGENT_LANGUAGE"] = c.AgentLanguage
	}
	if c.RepoLanguage != "" {
		env["REPO_LANGUAGE"] = c.RepoLanguage
	}
	if c.GitSigningKey != "" {
		env["GIT_SIGNING_KEY"] = c.GitSigningKey
	}

	return env
}
