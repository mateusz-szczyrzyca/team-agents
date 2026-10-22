package main

import (
	"strings"
	"time"
)

// Capability is a typed capability string formatted as <namespace>:<identifier>.
// Namespaces: os:, build:, test:, hw:, model:, sign:, publish:.
type Capability string

// ValidNamespaces lists the allowed capability namespace prefixes.
var ValidNamespaces = []string{
	"os:",
	"build:",
	"test:",
	"hw:",
	"model:",
	"sign:",
	"publish:",
}

// RecognizedCapabilities is the closed set of valid capability identifiers.
var RecognizedCapabilities = map[Capability]bool{
	"os:linux":                true,
	"os:darwin":               true,
	"build:ios":               true,
	"build:android":           true,
	"build:rust":              true,
	"build:go":                true,
	"build:c":                 true,
	"test:ios-simulator":      true,
	"test:android-emulator":   true,
	"test:docker":             true,
	"hw:rp2350-probe":         true,
	"hw:jlink":                true,
	"hw:apple-silicon":        true,
	"hw:x86-64":               true,
	"model:claude-3-7-sonnet": true,
	"model:codex":             true,
	"model:gemini-flash":      true,
	"sign:apple":              true,
	"sign:apple-development":  true,
	"sign:apple-distribution": true,
	"sign:android-keystore":   true,
	"publish:npm":             true,
	"publish:pypi":            true,
	"publish:github-release":  true,
}

// Valid reports whether c belongs to the closed set of recognized capabilities.
func (c Capability) Valid() bool {
	return RecognizedCapabilities[c]
}

// UnmarshalJSON implements json.Unmarshaler; failures contain *ValueError.
func (c *Capability) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, c, "capability").asError()
}

// Namespace returns the namespace prefix (including the colon) of the capability.
func (c Capability) Namespace() string {
	s := string(c)
	colon := strings.IndexByte(s, ':')
	if colon < 0 {
		return ""
	}
	return s[:colon+1]
}

// IsSensitive reports whether the capability requires operator approval before execution (sign:*, publish:*).
func (c Capability) IsSensitive() bool {
	ns := c.Namespace()
	return ns == "sign:" || ns == "publish:"
}

// ParseCapability validates that s is a known capability and returns it as a typed Capability.
func ParseCapability(s string) (Capability, *ValueError) {
	c, err := parseEnum[Capability](s, "capability")
	if err != nil {
		return "", err
	}
	return c, nil
}

// InstanceManifest is the payload sent by agent node during registration and heartbeat.
type InstanceManifest struct {
	InstanceName string               `json:"instance_name"` // unique identifier of the instance (e.g. "mac", "pi5")
	Timestamp    time.Time            `json:"timestamp"`     // generation timestamp
	TTLSeconds   int                  `json:"ttl_seconds"`   // time-to-live before instance is considered unreachable
	Detected     DetectedCapabilities `json:"detected"`      // discovered platform tools and hardware
	Declared     []Capability         `json:"declared"`      // declared capabilities from configuration
	CurrentState InstanceCurrentState `json:"current_state"` // active agents, slots, and quotas
}

// DetectedCapabilities describes automatically discovered platform tools and hardware.
type DetectedCapabilities struct {
	OS             string   `json:"os"`                        // operating system (e.g. "linux", "darwin")
	Arch           string   `json:"arch"`                      // cpu architecture (e.g. "amd64", "arm64")
	Toolchains     []string `json:"toolchains"`                // installed toolchains (e.g. ["go1.23", "rust1.82"])
	XcodeVersion   string   `json:"xcode_version,omitempty"`   // xcode version string if installed
	IOSSimulators  []string `json:"ios_simulators,omitempty"`  // installed ios simulator runtimes
	HardwareProbes []string `json:"hardware_probes,omitempty"` // attached hardware debug probes
}

// InstanceCurrentState describes the running agents, slots, and quotas on the instance.
type InstanceCurrentState struct {
	Slots         SlotCapacity                 `json:"slots"`           // total and free task runner slots
	Agents        []ManifestAgentState         `json:"agents"`          // list of agents running or configured
	Subscriptions map[string]SubscriptionQuota `json:"subscriptions"`   // provider subscription quota exhaustion state
	Tasks         map[string]HubTaskSummary    `json:"tasks,omitempty"` // active task summaries on this instance
}

// SlotCapacity reports total and free runner capacity.
type SlotCapacity struct {
	Total int `json:"total"` // total runner capacity
	Free  int `json:"free"`  // currently idle runner slots
}

// ManifestAgentState describes one agent on the instance.
type ManifestAgentState struct {
	Name        string `json:"name"`                   // agent name
	Tier        Tier   `json:"tier"`                   // normal or worker
	Role        string `json:"role"`                   // assigned role
	Status      string `json:"status"`                 // current activity status (idle, busy, sleeping)
	CurrentTask string `json:"current_task,omitempty"` // task id currently being executed
}

// SubscriptionQuota describes whether a provider quota is currently exhausted.
type SubscriptionQuota struct {
	Exhausted      bool       `json:"exhausted"`                 // whether quota is exhausted
	ExhaustedUntil *time.Time `json:"exhausted_until,omitempty"` // when quota resets if known
}

// HubTaskSummary summarizes the state of a task running on an instance.
type HubTaskSummary struct {
	Repo      string    `json:"repo,omitempty"`     // repository key for notebooks; absent in older manifests
	Branch    string    `json:"branch,omitempty"`   // task branch on its owning instance
	Worktree  string    `json:"worktree,omitempty"` // display metadata only for remote tasks
	Tier      Tier      `json:"tier,omitempty"`     // registered task tier; empty legacy summaries cannot authorize calls
	Role      Role      `json:"role,omitempty"`     // registered task role; empty legacy summaries cannot authorize calls
	TaskID    string    `json:"task_id"`            // task directory identifier
	Agent     string    `json:"agent"`              // executing agent
	Team      string    `json:"team"`               // team identifier if part of a team
	Status    string    `json:"status"`             // current runner state
	StartedAt time.Time `json:"started_at"`         // when task execution began
	UpdatedAt time.Time `json:"updated_at"`         // last activity timestamp
}
