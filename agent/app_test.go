package main

import (
	"bytes"
	"context"
	"reflect"
	"testing"
	"time"
)

// appClock supplies a fixed timestamp to exercise deterministic default seeding.
type appClock struct {
	now time.Time // timestamp returned on every call
}

// Now returns the injected timestamp.
func (c appClock) Now() time.Time { return c.now }

// appRandom supplies a recognizable draw without consulting process randomness.
type appRandom struct{}

// Int63 returns the injected draw.
func (appRandom) Int63() int64 { return 73 }

// appExecutor records the exact invocation and returns a controlled result.
type appExecutor struct {
	ctx    context.Context // context received by the boundary
	cmd    Command         // command received without shell interpretation
	calls  int             // number of observed executions
	result CommandResult   // controlled child result
	err    *ExecuteError   // controlled operation failure
}

// Execute records the call and returns the configured result and typed failure.
func (e *appExecutor) Execute(ctx context.Context, cmd Command) (CommandResult, *ExecuteError) {
	e.ctx, e.cmd = ctx, cmd
	e.calls++
	return e.result, e.err
}

// TestAppDefaultsDenyExecution verifies omitted dependencies never grant access.
func TestAppDefaultsDenyExecution(t *testing.T) {
	app := newApp(nil, nil, nil)
	if app == nil || app.Executor == nil || app.Clock == nil || app.Random == nil {
		t.Fatal("constructor must supply non-nil owned boundaries")
	}
	_, err := app.execute(context.Background(), Command{Name: "unexpected-provider"})
	if err == nil || err.Cause != ExecuteDenied {
		t.Fatalf("unexpected execution: got %v, want ExecuteDenied", err)
	}
}

// TestAppZeroValueDeniesExecution covers callers that bypass the constructor.
func TestAppZeroValueDeniesExecution(t *testing.T) {
	var app App
	_, err := app.execute(context.Background(), Command{Name: "unexpected-systemd"})
	if err == nil || err.Cause != ExecuteDenied {
		t.Fatalf("zero-value execution: got %v, want ExecuteDenied", err)
	}
}

// TestAppForwardsExecution verifies context, argv, I/O and failures stay intact.
func TestAppForwardsExecution(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	stdin := bytes.NewBufferString("input\n")
	var stdout, stderr bytes.Buffer
	cmd := Command{Name: "fake", Args: []string{"space arg", "line\nbreak"},
		Env: []string{"ONLY=test"}, Dir: t.TempDir(), Stdin: stdin,
		Stdout: &stdout, Stderr: &stderr}
	failure := opErr(ExecuteWait, "controlled failure")
	fake := &appExecutor{result: CommandResult{ExitCode: 27}, err: failure}
	app := newApp(fake, appClock{}, appRandom{})
	result, err := app.execute(ctx, cmd)
	if result.ExitCode != 27 || err != failure || fake.calls != 1 {
		t.Fatalf("forwarded result=%+v error=%v calls=%d", result, err, fake.calls)
	}
	if fake.ctx != ctx || !reflect.DeepEqual(fake.cmd, cmd) {
		t.Fatalf("invocation changed: got %+v, want %+v", fake.cmd, cmd)
	}
}

// TestAppUsesInjectedTimeAndRandom verifies neither source is silently replaced.
func TestAppUsesInjectedTimeAndRandom(t *testing.T) {
	clock := appClock{now: time.Unix(123, 456)}
	app := newApp(nil, clock, appRandom{})
	if app.Clock == nil || app.Random == nil {
		t.Fatal("injected sources were discarded")
	}
	if !app.Clock.Now().Equal(clock.now) || app.Random.Int63() != 73 {
		t.Fatal("injected time or random draw changed")
	}
}

// TestAppSeedsIndependentRandomFromClock verifies reproducible owned defaults.
func TestAppSeedsIndependentRandomFromClock(t *testing.T) {
	clock := appClock{now: time.Unix(789, 123)}
	first, second := newApp(nil, clock, nil), newApp(nil, clock, nil)
	if first.Random == nil || second.Random == nil {
		t.Fatal("default random source missing")
	}
	expected := newRandom(clock.now.UnixNano())
	for i := 0; i < 5; i++ {
		want := expected.Int63()
		if first.Random.Int63() != want || second.Random.Int63() != want {
			t.Fatal("default sources are not independently seeded from the injected clock")
		}
	}
}
