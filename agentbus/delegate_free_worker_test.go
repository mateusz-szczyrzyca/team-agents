package main

// Tests of the free worker a request of an agent with a plan needs (written first): in a team, such a request is
// refused at once when no worker of the team is free to take it, so that the requester does the step itself and no
// worker does it again later; a request in the old form still waits in the queue as before.

import (
	"strings"
	"testing"
)

// teamOf is a team T with the members given (name → tier and state).
func teamOf(members map[string][2]string) *Team {
	team := &Team{ID: "T", Members: map[string]*TeamMember{}}
	for name, v := range members {
		team.Members[name] = &TeamMember{Agent: name, Task: "task-" + name, Tier: Tier(v[0]), State: MemberState(v[1])}
	}
	return team
}

func TestFreeWorkersOfATeam(t *testing.T) {
	member := [2]string{"normal", "active"}
	cases := []struct {
		name    string
		members map[string][2]string
		helpers []*Helper
		want    int
	}{
		{"no worker", map[string][2]string{"m": member}, nil, 0},
		{"a sleeping worker", map[string][2]string{"m": member, "w": {"worker", "sleeping"}}, nil, 1},
		{"a worker in a run of its own is not free", map[string][2]string{"m": member, "w": {"worker", "active"}}, nil, 0},
		{"a waking worker is not free", map[string][2]string{"m": member, "w": {"worker", "waking"}}, nil, 0},
		{"a worker that serves a request", map[string][2]string{"m": member, "w": {"worker", "active"}},
			[]*Helper{{ID: 1, From: "m", Status: HelperRunning, Worker: "w"}}, 0},
		{"a worker that is starting a request", map[string][2]string{"m": member, "w": {"worker", "active"}},
			[]*Helper{{ID: 1, From: "m", Status: HelperStarting, Worker: "w"}}, 0},
		{"a request of the team waits for the one worker", map[string][2]string{"m": member, "w": {"worker", "sleeping"}},
			[]*Helper{{ID: 1, From: "m", Status: HelperQueued}}, 0},
		{"two workers, one request waiting", map[string][2]string{"m": member, "w": {"worker", "sleeping"},
			"v": {"worker", "sleeping"}}, []*Helper{{ID: 1, From: "m", Status: HelperQueued}}, 1},
		{"a request of another team does not count", map[string][2]string{"m": member, "w": {"worker", "sleeping"}},
			[]*Helper{{ID: 1, From: "stranger", Status: HelperQueued}}, 1},
		{"finished requests do not count", map[string][2]string{"m": member, "w": {"worker", "sleeping"}},
			[]*Helper{{ID: 1, From: "m", Status: HelperDone, Worker: "w"}, {ID: 2, From: "m", Status: HelperFailed, Worker: "w"},
				{ID: 3, From: "m", Status: HelperUnclear, Worker: "w"}}, 1},
		{"a worker that is done, excluded, hung, paused or stopped is not free", map[string][2]string{"m": member,
			"a": {"worker", "done"}, "b": {"worker", "excluded"}, "c": {"worker", "hung"}, "d": {"worker", "paused"},
			"e": {"worker", "stopped"}}, nil, 0},
	}
	for _, c := range cases {
		if got := freeWorkers(teamOf(c.members), c.helpers); got != c.want {
			t.Errorf("%s: freeWorkers = %d, want %d", c.name, got, c.want)
		}
	}
}

// planMember is the member m of team T, with a plan whose requests the harness accepts.
func planMember(t *testing.T, b *Bus, members map[string][3]string) *Server {
	t.Helper()
	createTeam(t, b, "T", members)
	m := server(b, "m", inTeam("T"))
	m.plan = planPath(t)
	m.agentCmd, m.home = "agent", "/home/fake"
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	return m
}

func TestDelegateWithAPlanIsRefusedWhenNoWorkerOfTheTeamIsFree(t *testing.T) {
	b := newBus(t)
	m := planMember(t, b, map[string][3]string{"m": {"normal", "a", "active"}})

	err := failure(t, DelegateNoFreeWorker)(m.delegateFields(namedFields(t.TempDir())))

	if !strings.Contains(err.Message, "1.1") {
		t.Fatalf("the refusal should name the step to do oneself: %q", err.Message)
	}
	mustWith(t, b, func(st *State) {
		if len(st.Helpers) != 0 {
			t.Fatalf("a refused request is not queued: %+v", st.Helpers)
		}
	})
}

func TestDelegateWithAPlanIsQueuedWhenAWorkerOfTheTeamIsFree(t *testing.T) {
	b := newBus(t)
	m := planMember(t, b, map[string][3]string{"m": {"normal", "a", "active"}, "w": {"worker", "z", "sleeping"}})

	chk(t).ok(toolResult(m.delegateFields(namedFields(t.TempDir()))))
	failsWith(t, DelegateNoFreeWorker)(m.delegateFields(namedFields(t.TempDir())))
}

func TestDelegateInTheOldFormStillWaitsWithoutAFreeWorker(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"m": {"normal", "a", "active"}})
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })

	chk(t).ok(toolResult(server(b, "m", inTeam("T")).delegateFields(fields(t.TempDir()))))
}
