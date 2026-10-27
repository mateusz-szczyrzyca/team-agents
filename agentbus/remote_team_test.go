package main

// Tests of a remote member in a team's life (written first, docs/tdd.md): its job is validated as a member's; going
// to sleep it goes through the same logic as a local member (woken for a waiting message, through its proxy task on
// the lead); a wake waits for its node until it runs; when its job ends it is done in its team, its branch is
// recorded and the coordinator is told.

import (
	"strings"
	"testing"
	"time"
)

// memberRequest is the job of solo@node2, a member of team T.
func memberRequest() JobRequest {
	req := regRequest("job-m")
	req.Selector, req.Target, req.Team, req.Member = "solo", "node2", "T", "solo@node2"
	return req
}

// remoteTeam is a lead with team T — lead (local) and solo@node2 (remote: proxy task p1, job job-m assigned to
// node2) — and returns the job's assignment.
func remoteTeam(t *testing.T) (*Bus, *leadAPI, *[]string, uint64) {
	t.Helper()
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "solo@node2": {"normal", "p1", "active"}})
	a := &leadAPI{bus: b, ttl: time.Minute}
	if _, e := a.submit("lead", true, memberRequest()); e != nil {
		t.Fatal(e)
	}
	job, e := a.heartbeat("node2", leadHeartbeatBody{NodeHeartbeat: regBeat("node2", nil, FreeAgent{Name: "solo", Tier: TierNormal})})
	if e != nil || job == nil {
		t.Fatalf("job-m not assigned to node2: %v %v", job, e)
	}
	if e := a.grantMember(RemoteMember{Member: "solo@node2", Agent: "solo", Node: "node2", Team: "T", Task: "p1", Tier: TierNormal, Job: "job-m"}); e != nil {
		t.Fatal(e)
	}
	return b, a, woken, job.Assignment
}

// memberState is a team member's state and branch on the lead's bus.
func memberState(t *testing.T, b *Bus, member string) (MemberState, string) {
	t.Helper()
	var st MemberState
	var branch string
	mustWith(t, b, func(s *State) {
		m := s.Teams["T"].Members[member]
		st, branch = m.State, m.Branch
	})
	return st, branch
}

func TestMemberJobRequestIsValidated(t *testing.T) {
	if _, err := validateJobRequest(memberRequest()); err != nil {
		t.Fatalf("a valid member job refused: %v", err)
	}
	for name, change := range map[string]func(*JobRequest){
		"member is not selector@target": func(r *JobRequest) { r.Member = "other@node2" },
		"no target":                     func(r *JobRequest) { r.Target = ""; r.Member = "solo@" },
		"team without member":           func(r *JobRequest) { r.Member = "" },
		"member without team":           func(r *JobRequest) { r.Team = "" },
		"selector is a tier":            func(r *JobRequest) { r.Selector, r.Member = "normal", "normal@node2" },
	} {
		req := memberRequest()
		change(&req)
		_, err := validateJobRequest(req)
		failsWith(t, LeadSubmitInvalidMember)(name, err)
	}
}

func TestRemoteMemberGoingToSleepIsWokenForAWaitingMessage(t *testing.T) {
	b, a, woken, _ := remoteTeam(t)
	var proxy string // the member's task in its team's record: its proxy task on the lead
	mustWith(t, b, func(s *State) { proxy = s.Teams["T"].Members["solo@node2"].Task })
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).send("solo@node2", "a question for you")))
	if _, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "sleeping"}}); err != nil {
		t.Fatal(err)
	}
	if len(*woken) != 1 || (*woken)[0] != proxy {
		t.Fatalf("woken %v, want its proxy task %s", *woken, proxy)
	}
}

func TestAWakeWaitsForTheNodeUntilTheMemberRuns(t *testing.T) {
	b, a, _, _ := remoteTeam(t)
	if err := a.wakeMember("solo@node2"); err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 2; i++ { // asked again while the member still sleeps
		wakes, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "sleeping"}})
		if err != nil || len(wakes) != 1 || wakes[0] != "job-m" {
			t.Fatalf("heartbeat %d: wakes %v, %v; want job-m", i, wakes, err)
		}
	}
	wakes, err := a.memberActivity("node2", []JobActivity{{ID: "job-m", Agent: "solo", Stage: "running"}})
	if err != nil || len(wakes) != 0 {
		t.Fatalf("a running member is still to be woken: %v %v", wakes, err)
	}
	if st, _ := memberState(t, b, "solo@node2"); st != MemberActive {
		t.Errorf("a running remote member is %s, want active", st)
	}
	failsWith(t, LeadMemberNoSuchMember)("unknown", a.wakeMember("nobody@node2"))
}

func TestFinishedRemoteMemberIsDoneAndTheCoordinatorIsTold(t *testing.T) {
	b, a, _, assignment := remoteTeam(t)
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).claimLead()))
	for _, rep := range []JobReport{
		{ID: "job-m", Assignment: assignment, State: JobRunning, TaskID: "t-node", Branch: "agents/solo/x"},
		{ID: "job-m", Assignment: assignment, State: JobDone, TaskID: "t-node", Branch: "agents/solo/x", Result: "all done"},
	} {
		if _, err := a.report("node2", "job-m", rep); err != nil {
			t.Fatal(err)
		}
	}
	st, branch := memberState(t, b, "solo@node2")
	if st != MemberDone || branch != "agents/solo/x" {
		t.Errorf("the finished remote member is %s on %q, want done on agents/solo/x", st, branch)
	}
	out := chk(t).ok(toolResult(server(b, "lead", inTeam("T")).read(0)))
	if !strings.Contains(out, "solo@node2") || !strings.Contains(out, "finished") {
		t.Errorf("the coordinator was not told:\n%s", out)
	}
}
