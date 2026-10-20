// Typed values of the shared state.
//
// Statuses, tiers and levels are string-backed types with a fixed set of constants. They keep the on-disk JSON
// readable and compatible with the `agent` script and agentlive ("status": "running"), but an unknown value
// is rejected when state.json is decoded instead of silently flowing through the code. Timestamps are a
// Timestamp type whose zero value means "has not happened yet" (e.g. an investigation that has not started).
package main

import (
	"encoding/json"
	"time"
)

// ---------------------------------------------------------------- generic helpers

// enumValue is implemented by every string-backed enum of this file.
type enumValue interface {
	~string
	Valid() bool
}

// unmarshalEnum decodes a JSON string into an enum. It fails only with a *ValueError: the value is not a
// JSON string, or it is not one of the enum's constants.
func unmarshalEnum[T enumValue](data []byte, dst *T, typeName string) *ValueError {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return valueWrap(err, "%s must be a JSON string", typeName)
	}
	v := T(s)
	if !v.Valid() {
		return valueErr("unknown %s %q", typeName, s)
	}
	*dst = v
	return nil
}

// ---------------------------------------------------------------- investigations

// InvestigationStatus is the lifecycle state of an investigation (or PR review).
//
//	queued | waitroom → starting → running ⇄ waiting_human → done | failed
//	rejected: the queue was full and the requester did not choose to wait
type InvestigationStatus string

const (
	InvQueued       InvestigationStatus = "queued"        // in the queue, will start when an investigator is free
	InvWaitroom     InvestigationStatus = "waitroom"      // the queue was full; enters the queue when there is room
	InvStarting     InvestigationStatus = "starting"      // picked by the dispatcher, the investigator run is being created
	InvRunning      InvestigationStatus = "running"       // an investigator is working on it
	InvWaitingHuman InvestigationStatus = "waiting_human" // the investigator asked the human and waits for answers
	InvDone         InvestigationStatus = "done"          // finished with a result
	InvFailed       InvestigationStatus = "failed"        // finished without a usable result (or cancelled)
	InvRejected     InvestigationStatus = "rejected"      // not accepted because the queue was full
)

// Valid reports whether s is one of the defined statuses.
func (s InvestigationStatus) Valid() bool {
	switch s {
	case InvQueued, InvWaitroom, InvStarting, InvRunning, InvWaitingHuman, InvDone, InvFailed, InvRejected:
		return true
	}
	return false
}

// Pending reports whether the investigation has not finished yet (its requester still waits for it).
func (s InvestigationStatus) Pending() bool {
	return s == InvQueued || s == InvWaitroom || s.Active()
}

// Active reports whether the investigation occupies an investigator slot.
func (s InvestigationStatus) Active() bool {
	return s == InvStarting || s == InvRunning || s == InvWaitingHuman
}

// Finished reports whether the status is final.
func (s InvestigationStatus) Finished() bool {
	return s == InvDone || s == InvFailed || s == InvRejected
}

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown statuses with a *ValueError.
func (s *InvestigationStatus) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, s, "investigation status").asError()
}

// InvestigationKind distinguishes investigations requested by agents from PR reviews queued by --verify.
type InvestigationKind string

const (
	KindInvestigation InvestigationKind = ""       // an agent asked for help (request_investigation)
	KindReview        InvestigationKind = "review" // a PR review of a finished team (agent run --team --verify)
)

// Valid reports whether k is one of the defined kinds.
func (k InvestigationKind) Valid() bool { return k == KindInvestigation || k == KindReview }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown kinds with a *ValueError.
func (k *InvestigationKind) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, k, "investigation kind").asError()
}

// ---------------------------------------------------------------- worker requests

// HelperStatus is the lifecycle state of a worker request (delegate).
type HelperStatus string

const (
	HelperQueued   HelperStatus = "queued"   // waiting for a free worker
	HelperStarting HelperStatus = "starting" // assigned to a worker, its helper task is being created
	HelperRunning  HelperStatus = "running"  // the worker is working on it
	HelperDone     HelperStatus = "done"     // the result was sent to the requester
	HelperFailed   HelperStatus = "failed"   // the worker failed, hung or the request was cancelled
)

// Valid reports whether s is one of the defined statuses.
func (s HelperStatus) Valid() bool {
	switch s {
	case HelperQueued, HelperStarting, HelperRunning, HelperDone, HelperFailed:
		return true
	}
	return false
}

// Open reports whether the request has not finished yet.
func (s HelperStatus) Open() bool {
	return s == HelperQueued || s == HelperStarting || s == HelperRunning
}

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown statuses with a *ValueError.
func (s *HelperStatus) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, s, "worker request status").asError()
}

// ---------------------------------------------------------------- teams

// Tier is an agent's model tier.
type Tier string

const (
	TierNormal Tier = "normal" // does the real work, may coordinate a team and delegate
	TierWorker Tier = "worker" // a cheap model: easy board items and delegate requests only
)

// Valid reports whether t is one of the defined tiers.
func (t Tier) Valid() bool { return t == TierNormal || t == TierWorker }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown tiers with a *ValueError.
func (t *Tier) UnmarshalJSON(data []byte) error { return unmarshalEnum(data, t, "tier").asError() }

// MemberState is the state of a team member.
type MemberState string

const (
	MemberActive   MemberState = "active"   // its run is in progress
	MemberSleeping MemberState = "sleeping" // waiting for messages or board items (no run, no tokens)
	MemberWaking   MemberState = "waking"   // a wake-up was triggered, the run is starting
	MemberDone     MemberState = "done"     // finished (with the team or on its own)
	MemberExcluded MemberState = "excluded" // failed the quiz or was excluded by the coordinator
	MemberHung     MemberState = "hung"     // marked hung by the watchdog (no longer resumed)
)

// Valid reports whether s is one of the defined states.
func (s MemberState) Valid() bool {
	switch s {
	case MemberActive, MemberSleeping, MemberWaking, MemberDone, MemberExcluded, MemberHung:
		return true
	}
	return false
}

// Working reports whether the member is running or about to run (used for provider load balancing).
func (s MemberState) Working() bool { return s == MemberActive || s == MemberWaking }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown states with a *ValueError.
func (s *MemberState) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, s, "member state").asError()
}

// BoardLevel says who should take a board item.
type BoardLevel string

const (
	LevelEasy   BoardLevel = "easy"   // simple, mechanical work for workers
	LevelNormal BoardLevel = "normal" // work that needs reasoning, for normal models
)

// Valid reports whether l is one of the defined levels.
func (l BoardLevel) Valid() bool { return l == LevelEasy || l == LevelNormal }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown levels with a *ValueError.
func (l *BoardLevel) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, l, "board level").asError()
}

// parseBoardLevel converts a tool argument into a BoardLevel.
func parseBoardLevel(s string) (BoardLevel, *ValueError) {
	l := BoardLevel(s)
	if !l.Valid() {
		return "", valueErr("level must be %q or %q, got %q", LevelEasy, LevelNormal, s)
	}
	return l, nil
}

// BoardStatus is the state of a board item.
type BoardStatus string

const (
	BoardTodo    BoardStatus = "todo"    // waiting for someone to claim it
	BoardClaimed BoardStatus = "claimed" // someone is working on it
	BoardDone    BoardStatus = "done"    // the result was submitted
)

// Valid reports whether s is one of the defined statuses.
func (s BoardStatus) Valid() bool { return s == BoardTodo || s == BoardClaimed || s == BoardDone }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) rejects unknown statuses with a *ValueError.
func (s *BoardStatus) UnmarshalJSON(data []byte) error {
	return unmarshalEnum(data, s, "board status").asError()
}

// Role is what an agentbus serve instance serves.
type Role string

const (
	RoleAgent        Role = "worker"       // a regular agent run (the flag value "worker" is historical)
	RoleInvestigator Role = "investigator" // an investigator run: no investigation tools
)

// Valid reports whether r is one of the defined roles.
func (r Role) Valid() bool { return r == RoleAgent || r == RoleInvestigator }

// ---------------------------------------------------------------- timestamps

// timestampLayout is the on-disk format of every timestamp (local time, second precision).
const timestampLayout = "2006-01-02 15:04:05"

// Timestamp is a point in time stored as "2006-01-02 15:04:05" (local time). The zero value means "not yet"
// (it is stored as an empty string), so `t.IsZero()` replaces checks like `inv.Started == ""`.
type Timestamp struct {
	time.Time // the instant; the zero time means "not yet"
}

// Now returns the current time truncated to the stored precision.
func Now() Timestamp { return Timestamp{time.Now().Truncate(time.Second)} }

// String formats the timestamp as stored ("" for the zero value).
func (t Timestamp) String() string {
	if t.IsZero() {
		return ""
	}
	return t.Format(timestampLayout)
}

// MarshalJSON stores the timestamp as a string ("" for the zero value).
func (t Timestamp) MarshalJSON() ([]byte, error) { return json.Marshal(t.String()) }

// UnmarshalJSON (json.Unmarshaler, hence the plain error type) accepts "" (zero), the storage layout and
// RFC 3339; anything else is a *ValueError.
func (t *Timestamp) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return valueWrap(err, "a timestamp must be a JSON string").asError()
	}
	if s == "" {
		*t = Timestamp{}
		return nil
	}
	if v, err := time.ParseInLocation(timestampLayout, s, time.Local); err == nil {
		*t = Timestamp{v}
		return nil
	}
	v, err := time.Parse(time.RFC3339, s)
	if err != nil {
		return valueWrap(err, "invalid timestamp %q", s).asError()
	}
	*t = Timestamp{v}
	return nil
}

// parseEnum converts a string (a flag or tool argument) into an enum value, rejecting unknown values.
func parseEnum[T enumValue](s, typeName string) (T, *ValueError) {
	v := T(s)
	if !v.Valid() {
		return v, valueErr("unknown %s %q", typeName, s)
	}
	return v, nil
}
