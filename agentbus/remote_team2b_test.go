package main

// Tests of part 2b (written first, docs/tdd.md): a remote member's changes reach the team's logic in the agent
// script (asleep, at work, its job's end with its branch); the doorbell rings a node when something waits for it —
// and not on an ordinary heartbeat.

import (
	"bufio"
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// captureRemoteMember records the remote-member changes the lead hands to the agent script: "<task> <status> <branch>".
func captureRemoteMember(t *testing.T) *[]string {
	t.Helper()
	var mu sync.Mutex
	got := []string{}
	old := remoteMemberFn
	remoteMemberFn = func(_ *Team, task, status, branch, _ string) {
		mu.Lock()
		defer mu.Unlock()
		got = append(got, task+" "+status+" "+branch)
	}
	t.Cleanup(func() { remoteMemberFn = old })
	return &got
}

func TestRemoteMemberChangesReachTheTeamLogic(t *testing.T) {
	b, a, _, assignment := remoteTeam(t)
	events := captureRemoteMember(t)
	var proxy string // the member's proxy task on the lead
	mustWith(t, b, func(s *State) { proxy = s.Teams["T"].Members["solo@node2"].Task })
	for _, stage := range []string{"sleeping", "sleeping", "running"} { // a repeated stage is no change
		if _, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: stage}}); err != nil {
			t.Fatal(err)
		}
	}
	for _, rep := range []JobReport{
		{ID: "job-m", Assignment: assignment, State: JobRunning, TaskID: "t-node", Branch: "agents/solo/x"},
		{ID: "job-m", Assignment: assignment, State: JobFailed, TaskID: "t-node", Branch: "agents/solo/x", Result: "boom"},
	} {
		if _, err := a.report("node2", "job-m", rep); err != nil {
			t.Fatal(err)
		}
	}
	want := []string{proxy + " sleeping ", proxy + " running ", proxy + " failed agents/solo/x"}
	if strings.Join(*events, "|") != strings.Join(want, "|") {
		t.Errorf("events %q, want %q", *events, want)
	}
}

// rings reads a node's event stream and reports each ring on the channel.
func rings(t *testing.T, url string) <-chan struct{} {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK || !strings.HasPrefix(resp.Header.Get("Content-Type"), "text/event-stream") {
		t.Fatalf("events: HTTP %d %s", resp.StatusCode, resp.Header.Get("Content-Type"))
	}
	ch := make(chan struct{}, 8)
	go func() {
		defer func() {
			if cerr := resp.Body.Close(); cerr != nil && ctx.Err() == nil {
				t.Errorf("close the event stream: %v", cerr)
			}
		}()
		sc := bufio.NewScanner(resp.Body)
		for sc.Scan() {
			if sc.Text() == "event: ring" {
				ch <- struct{}{}
			}
		}
	}()
	return ch
}

// rang reports whether a ring comes within d.
func rang(ch <-chan struct{}, d time.Duration) bool {
	select {
	case <-ch:
		return true
	case <-time.After(d):
		return false
	}
}

func TestDoorbellRingsWhenSomethingWaitsForTheNode(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	captureRemoteMember(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "solo@node-a": {"normal", "p", "active"}})
	a := &leadAPI{bus: b, ttl: time.Minute}
	if err := a.grantMember(RemoteMember{Member: "solo@node-a", Agent: "solo", Node: "node-a", Team: "T", Task: "p", Tier: TierNormal, Job: "job-x"}); err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(newLeadHandler(b, "node-a", nil, time.Minute)) // loopback is node-a
	t.Cleanup(ts.Close)
	ch := rings(t, ts.URL+"/v1/events")
	if rang(ch, 500*time.Millisecond) {
		t.Fatal("rang with nothing waiting")
	}
	if err := a.wakeMember("solo@node-a"); err != nil {
		t.Fatal(err)
	}
	if !rang(ch, 2*time.Second) {
		t.Fatal("no ring for a waiting wake")
	}
	if _, err := a.heartbeat("node-a", leadHeartbeatBody{NodeHeartbeat: regBeat("node-a", nil)}); err != nil {
		t.Fatal(err)
	}
	if rang(ch, 800*time.Millisecond) {
		t.Error("rang again for a heartbeat: nothing new waits")
	}
}
