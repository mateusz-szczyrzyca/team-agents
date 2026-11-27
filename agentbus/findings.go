// Repository findings and their append-only journal.
package main

// Finding is an open note tied to repository paths.
type Finding struct {
	ID     string      `json:"id"`     // Stable F-number.
	TS     Timestamp   `json:"ts"`     // Addition time.
	Author string      `json:"author"` // Agent that added the note.
	Task   string      `json:"task"`   // Task that added the note.
	Kind   FindingKind `json:"kind"`   // Category of the note.
	Paths  []string    `json:"paths"`  // Normalized repository paths.
	Text   string      `json:"text"`   // Note for later agents.
}

// findingAdd stores a finding.
func (s *Server) findingAdd(kind string, paths []string, text string) (string, *FindingAddError) {
	return "", nil
}

// findingResolve closes a finding.
func (s *Server) findingResolve(id, reason string) (string, *FindingResolveError) { return "", nil }

// findings lists open findings under the bus lock.
func (b *Bus) findings(repo string) ([]Finding, *FindingsError) { return []Finding{}, nil }
