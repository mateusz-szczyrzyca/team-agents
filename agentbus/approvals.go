package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
)

// approvedCapabilitiesFile is the operator-maintained list of sensitive capabilities an instance may use, relative to
// its AGENTS_HOME. The node executor reads the same file. The operator writes it by hand: an agent that can edit it
// could approve itself, which is why the README states that agents on the same account must not be trusted with it.
const approvedCapabilitiesFile = "node/approved-capabilities.json"

// approvedCapabilities returns the sensitive capabilities approved for the instance whose AGENTS_HOME is home.
// A missing file approves nothing. An empty home (servers run by the hub have no local home) also approves nothing,
// so the gate fails closed. A file that cannot be read or decoded is a storage failure; a claim with no sensitive
// requirement never reads the file (see boardClaim).
func approvedCapabilities(home string) (map[Capability]bool, *StorageError) {
	approved := map[Capability]bool{}
	if home == "" {
		return approved, nil
	}
	data, err := os.ReadFile(filepath.Join(home, approvedCapabilitiesFile)) //nolint:gosec // G304: fixed filename under operator-selected AGENTS_HOME, never a request-supplied path
	if errors.Is(err, os.ErrNotExist) {
		return approved, nil
	}
	if err != nil {
		return nil, storageErr(err, "cannot read %s", approvedCapabilitiesFile)
	}
	var list []Capability
	if err := json.Unmarshal(data, &list); err != nil {
		return nil, storageErr(err, "cannot decode %s (expected a JSON array of capability names)", approvedCapabilitiesFile)
	}
	for _, c := range list {
		approved[c] = true
	}
	return approved, nil
}

// sensitiveOf returns the required capabilities that need operator approval.
func sensitiveOf(required []Capability) []Capability {
	var sensitive []Capability
	for _, c := range required {
		if c.IsSensitive() {
			sensitive = append(sensitive, c)
		}
	}
	return sensitive
}

// unapproved returns the sensitive capabilities that are not in approved.
func unapproved(sensitive []Capability, approved map[Capability]bool) []Capability {
	var missing []Capability
	for _, c := range sensitive {
		if !approved[c] {
			missing = append(missing, c)
		}
	}
	return missing
}
