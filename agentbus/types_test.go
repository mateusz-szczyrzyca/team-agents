package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestUnknownEnumValuesAreRejected(t *testing.T) {
	cases := map[string]any{
		`{"status":"bogus"}`:             &Investigation{},
		`{"state":"napping"}`:            &TeamMember{},
		`{"tier":"boss"}`:                &TeamMember{},
		`{"level":"hard"}`:               &BoardItem{},
		`{"status":"maybe"}`:             &BoardItem{},
		`{"status":"lost"}`:              &Helper{},
		`{"kind":"audit"}`:               &Investigation{},
		`{"status":7}`:                   &Investigation{},
		`{"created":"yesterday"}`:        &Investigation{},
		`{"created":"2026-13-45 99:00"}`: &Team{},
	}
	for in, dst := range cases {
		err := json.Unmarshal([]byte(in), dst)
		var verr *ValueError
		if !errors.As(err, &verr) || !errors.Is(err, ErrInvalidArgument) {
			t.Errorf("%s into %T: want a *ValueError, got %v", in, dst, err)
		}
	}
}

func TestUnmarshalEnumReturnsAValueError(t *testing.T) {
	var s MemberState
	verr := unmarshalEnum([]byte(`"napping"`), &s, "member state")
	if verr == nil || verr.Message != `unknown member state "napping"` || verr.Class() != ClassInvalidArgument {
		t.Fatalf("got %#v", verr)
	}
	if verr := unmarshalEnum([]byte(`"sleeping"`), &s, "member state"); verr != nil || s != MemberSleeping {
		t.Fatalf("a valid value: %v, %q", verr, s)
	}
}

func TestKnownEnumValuesAreAccepted(t *testing.T) {
	var inv Investigation
	in := `{"status":"waiting_human","kind":"review","created":"2026-10-20 12:00:00","started":""}`
	if err := json.Unmarshal([]byte(in), &inv); err != nil {
		t.Fatal(err)
	}
	if inv.Status != InvWaitingHuman || inv.Kind != KindReview || inv.Created.IsZero() || inv.HasStarted() {
		t.Fatalf("decoded %+v", inv)
	}
}

func TestCorruptStatusInStateFileIsAStorageError(t *testing.T) {
	b := newBus(t)
	state := `{"investigations":{"1":{"id":1,"status":"exploded"}}}`
	if err := os.WriteFile(filepath.Join(b.dir, "state.json"), []byte(state), 0o600); err != nil {
		t.Fatal(err)
	}
	serr := b.locked(func(*State) {})
	// the state could not be loaded (StorageError) because a value was invalid (ValueError inside)
	var verr *ValueError
	if serr == nil || !errors.Is(serr, ErrStorage) || !errors.As(serr, &verr) {
		t.Fatalf("want a StorageError wrapping a ValueError, got %v", serr)
	}
}

func TestTimestampJSON(t *testing.T) {
	var zero Timestamp
	if b, err := json.Marshal(zero); err != nil || string(b) != `""` {
		t.Fatalf("zero timestamp encodes as %s (%v)", b, err)
	}
	at := Timestamp{time.Date(2026, 10, 20, 12, 30, 5, 0, time.Local)}
	b, err := json.Marshal(at)
	if err != nil || string(b) != `"2026-10-20 12:30:05"` {
		t.Fatalf("encodes as %s (%v)", b, err)
	}
	var back Timestamp
	if err := json.Unmarshal(b, &back); err != nil || !back.Equal(at.Time) {
		t.Fatalf("round trip: %v (%v)", back, err)
	}
	var rfc Timestamp
	if err := json.Unmarshal([]byte(`"2026-10-20T10:30:05Z"`), &rfc); err != nil || rfc.IsZero() {
		t.Fatalf("RFC 3339 must be accepted: %v (%v)", rfc, err)
	}
}

func TestInvestigationStatusPredicates(t *testing.T) {
	type want struct{ pending, active, finished bool }
	cases := map[InvestigationStatus]want{
		InvQueued: {true, false, false}, InvWaitroom: {true, false, false},
		InvStarting: {true, true, false}, InvRunning: {true, true, false}, InvWaitingHuman: {true, true, false},
		InvDone: {false, false, true}, InvFailed: {false, false, true}, InvRejected: {false, false, true},
	}
	for s, w := range cases {
		if got := (want{s.Pending(), s.Active(), s.Finished()}); got != w {
			t.Errorf("%s: got %+v, want %+v", s, got, w)
		}
	}
}

func TestParseEnum(t *testing.T) {
	if v, verr := parseEnum[MemberState]("sleeping", "member state"); verr != nil || v != MemberSleeping {
		t.Fatalf("got %q, %v", v, verr)
	}
	if _, verr := parseEnum[MemberState]("dreaming", "member state"); verr == nil || verr.Message != `unknown member state "dreaming"` {
		t.Fatalf("got %v", verr)
	}
	if _, verr := parseBoardLevel("hard"); verr == nil || !errors.Is(verr, ErrInvalidArgument) {
		t.Fatalf("board level: %v", verr)
	}
}

func TestToolArgumentsAreValidatedAgainstTheSchema(t *testing.T) {
	s := server(newBus(t), "a")
	cases := []struct {
		tool string
		args map[string]any
		want ToolCallCause
	}{
		{"read_messages", map[string]any{"history": "many"}, ToolCallInvalidArgument},
		{"read_messages", map[string]any{"history": 2.5}, ToolCallInvalidArgument},
		{"request_investigation", map[string]any{"question": "q", "wait_if_full": "yes"}, ToolCallInvalidArgument},
		{"send_message", map[string]any{"to": 7, "text": "x"}, ToolCallInvalidArgument},
		{"no_such_tool", map[string]any{}, ToolCallUnknownTool},
	}
	for _, c := range cases {
		_, err := s.call(c.tool, c.args)
		expectToolError(t, err, c.want)
	}
	// numbers and booleans may also arrive as strings
	if _, err := s.call("read_messages", map[string]any{"history": "3"}); err != nil {
		t.Fatalf("a numeric string must be accepted: %v", err)
	}
}

// osWriteFile writes a test file with private permissions.
func osWriteFile(path string, data []byte) error { return os.WriteFile(path, data, 0o600) }
