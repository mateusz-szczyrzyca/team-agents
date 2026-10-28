package main

import "encoding/json"

// SyncMetadataKey is one permitted display-only task metadata key.
type SyncMetadataKey string

// SyncFilename is a permitted task artifact name, never a network-selected path.
type SyncFilename string

// leadSyncFile carries the bounded UTF-8 representation and its content digest.
type leadSyncFile struct {
	Name    string // allow-listed basename
	Content string // exact bytes written to the mirror
	SHA256  string // lowercase SHA-256 of Content
	Cut     bool   // the node truncated the original artifact
}

// leadSyncBody updates one assigned job's task and appends bounded node logs.
type leadSyncBody struct {
	Job        string                     // existing job identifier
	Assignment uint64                     // current assignment fence
	TaskID     string                     // node-local task recorded by the accepted start report
	Metadata   map[SyncMetadataKey]string // only changed display metadata
	Files      []leadSyncFile             // only changed task artifacts
	Logs       []json.RawMessage          // new structured node log records
}

// sync stores validated data using the registry's existing lock and authorization.
func (a *leadAPI) sync(_ string, _ leadSyncBody) *LeadSyncError { return nil }
