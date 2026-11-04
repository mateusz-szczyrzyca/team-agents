package main

import (
	"context"
	"io"
)

// Config holds loaded agent runtime configuration.
// Scalars are simple key-value pairs. Indexed maps use numeric keys.
// Associative maps use string keys. Unset values are absent from maps;
// empty values are present with empty strings.
type Config struct {
	Scalars     map[string]string            // simple variables (FOO=bar)
	Indexed     map[string]map[string]string // indexed arrays ARR[0]=val
	Associative map[string]map[string]string // associative arrays MAP[key]=val
}

// ConfigContext provides explicit inputs that would otherwise come from the environment.
// Home, AgentsHome, Hostname, UID, and Self are passed to the config script.
type ConfigContext struct {
	Home       string // home directory (replaces $HOME)
	AgentsHome string // agents home (replaces $AGENTS_HOME)
	Hostname   string // hostname (replaces $HOSTNAME)
	UID        string // user ID as decimal string (replaces $UID)
	Self       string // self executable path (replaces $AGENT_BIN_SELF)
}

// ConfigRequest specifies how to load configuration.
type ConfigRequest struct {
	File    string        // path to agents.conf file
	Env     []string      // explicit environment variables (name=value format)
	Context ConfigContext // explicit context values
}

// loadShellConfig executes the Bash configuration sourcing and parses the result.
// It runs the configuration file through Bash with the given environment and context,
// reading NUL-delimited output from file descriptor 3 (FD3).
// The output is parsed into Config.Scalars, Config.Indexed, and Config.Associative maps.
// Unset variables are absent from maps; empty variables are present with empty values.
//
//nolint:unused,unparam // RED phase stub; called by tests in GREEN; error return nil until GREEN
func (a *App) loadShellConfig(ctx context.Context, req ConfigRequest) (Config, error) {
	// Stub implementation - to be filled in during GREEN phase
	_ = ctx
	_ = req
	_ = a
	return Config{
		Scalars:     make(map[string]string),
		Indexed:     make(map[string]map[string]string),
		Associative: make(map[string]map[string]string),
	}, nil
}

// decodeNULDelimited reads a map of key-value pairs from NUL-delimited data.
// The format is: key\x00value\x00key\x00value\x00...
// Empty values are preserved as empty strings.
// Returns an error if the data is malformed (odd number of fields, missing terminator).
func decodeNULDelimited(r io.Reader) (map[string]string, error) {
	// Stub implementation - to be filled in during GREEN phase
	_ = r
	return make(map[string]string), nil
}
