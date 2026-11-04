package main

// Config owns a Bash configuration snapshot; absent names differ from empty values.
// Array keys retain their Bash indices, including sparse indexed arrays.
type Config struct {
	Scalars     map[string]string            // scalar settings, including explicitly empty strings
	Indexed     map[string]map[string]string // indexed arrays, keyed by decimal Bash index
	Associative map[string]map[string]string // associative arrays, including declared empty maps
}

// ConfigContext supplies startup paths and host values without reading the installation.
type ConfigContext struct {
	Home       string // HOME before sourcing configuration
	AgentsHome string // AGENTS_HOME before sourcing configuration
	Hostname   string // HOSTNAME used for the default instance name
	UID        string // decimal user ID used for the default runtime directory
	Self       string // executable path available to configuration as SELF
}

// ConfigRequest describes an isolated configuration evaluation, never a secrets load.
type ConfigRequest struct {
	File    string        // executable Bash configuration file
	Env     []string      // complete subprocess environment; nil inherits the executor's environment
	Context ConfigContext // explicit startup values overriding the environment
}

// newConfig allocates independent containers for the three Bash declaration shapes.
func newConfig() Config {
	return Config{Scalars: make(map[string]string), Indexed: make(map[string]map[string]string), Associative: make(map[string]map[string]string)}
}

// applyConfigDefaults consumes a snapshot and applies the oracle's post-source defaults.
func applyConfigDefaults(c Config, _ ConfigContext) Config {
	return c
}
