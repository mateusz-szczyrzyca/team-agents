package main

// Tests of the free worker a request of an agent with a plan needs (written first): in a team, such a request is
// refused at once when no worker of the team is free to take it, so that the requester does the step itself and no
// worker does it again later; a request in the old form still waits in the queue as before.

import (
	"os"
	"path/filepath"
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

// teamWithTasks is team T whose members have the tasks given (name → task, tier and state).
func teamWithTasks(members map[string][3]string) *Team {
	team := &Team{ID: "T", Members: map[string]*TeamMember{}}
	for name, v := range members {
		team.Members[name] = &TeamMember{Agent: name, Task: v[0], Tier: Tier(v[1]), State: MemberState(v[2])}
	}
	return team
}

func TestFreeWorkersCountOnlyTheTeamsOwnRequests(t *testing.T) {
	// m and w are in team T (tasks task-m-t, task-w-t) and in another team U (tasks task-m-u, task-w-u).
	members := map[string][3]string{"m": {"task-m-t", "normal", "active"}, "w": {"task-w-t", "worker", "sleeping"}}
	cases := []struct {
		name    string
		helpers []*Helper
		want    int
	}{
		{"a request of the requester's task in another team does not count",
			[]*Helper{{ID: 1, From: "m", FromTask: "task-m-u", Status: HelperQueued}}, 1},
		{"a request of the requester's task in this team counts",
			[]*Helper{{ID: 1, From: "m", FromTask: "task-m-t", Status: HelperQueued}}, 0},
		{"a request recorded without its task counts by its requester",
			[]*Helper{{ID: 1, From: "m", Status: HelperQueued}}, 0},
		{"the worker serving in another team's task is free here",
			[]*Helper{{ID: 1, From: "x", FromTask: "task-x-u", Status: HelperRunning, Worker: "w", HelperTask: "task-w-u"}}, 1},
		{"the worker serving in this team's task is not free",
			[]*Helper{{ID: 1, From: "m", FromTask: "task-m-t", Status: HelperRunning, Worker: "w", HelperTask: "task-w-t"}}, 0},
		{"a worker that is starting a request, its task not yet recorded, is not free",
			[]*Helper{{ID: 1, From: "m", FromTask: "task-m-t", Status: HelperStarting, Worker: "w"}}, 0},
	}
	for _, c := range cases {
		if got := freeWorkers(teamWithTasks(members), c.helpers); got != c.want {
			t.Errorf("%s: freeWorkers = %d, want %d", c.name, got, c.want)
		}
	}
}

func TestFreeWorkersOfPool(t *testing.T) {
	pool := []string{"w1", "w2"}
	cases := []struct {
		name    string
		helpers []*Helper
		want    int
	}{
		{"two workers, no helpers", nil, 2},
		{"one worker running", []*Helper{{ID: 1, Status: HelperRunning, Worker: "w1"}}, 1},
		{"one worker starting", []*Helper{{ID: 1, Status: HelperStarting, Worker: "w1"}}, 1},
		{"one request queued", []*Helper{{ID: 1, Status: HelperQueued}}, 1},
		{"one running, one queued", []*Helper{{ID: 1, Status: HelperRunning, Worker: "w1"}, {ID: 2, Status: HelperQueued}}, 0},
		{"two running", []*Helper{{ID: 1, Status: HelperRunning, Worker: "w1"}, {ID: 2, Status: HelperRunning, Worker: "w2"}}, 0},
		{"finished helpers do not count", []*Helper{
			{ID: 1, Status: HelperDone, Worker: "w1"},
			{ID: 2, Status: HelperFailed, Worker: "w2"},
			{ID: 3, Status: HelperUnclear, Worker: "w1"},
		}, 2},
	}
	for _, c := range cases {
		if got := freeWorkers(nil, c.helpers, pool...); got != c.want {
			t.Errorf("%s: freeWorkers(nil, helpers, pool...) = %d, want %d", c.name, got, c.want)
		}
	}
	if got := freeWorkers(nil, nil); got != 0 {
		t.Errorf("empty pool: freeWorkers(nil, nil) = %d, want 0", got)
	}
}

func TestDelegateWithAPlanIsQueuedWhenPoolWorkerIsFree(t *testing.T) {
	b := newBus(t)
	// Team without workers: pool workers passed via s.workers.
	m := planMember(t, b, map[string][3]string{"m": {"normal", "a", "active"}})
	m.workers = []string{"pool-w1"}

	chk(t).ok(toolResult(m.delegateFields(namedFields(t.TempDir()))))
	err := failure(t, DelegateNoFreeWorker)(m.delegateFields(namedFields(t.TempDir())))
	if !strings.Contains(err.Message, "no worker of the pool is free") {
		t.Fatalf("expected pool refusal message, got: %q", err.Message)
	}
}

func TestDelegateWithAPlanSoloRequesterWithPoolWorkers(t *testing.T) {
	b := newBus(t)
	m := server(b, "solo")
	m.plan = planPath(t)
	m.agentCmd, m.home = "agent", "/home/fake"
	m.workers = []string{"pool-w1"}
	harnessCheck(t, func(string) string { return acceptedAnswer })
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })

	chk(t).ok(toolResult(m.delegateFields(namedFields(t.TempDir()))))
	err := failure(t, DelegateNoFreeWorker)(m.delegateFields(namedFields(t.TempDir())))
	if !strings.Contains(err.Message, "no worker of the pool is free") {
		t.Fatalf("expected pool refusal message, got: %q", err.Message)
	}
}

func TestHelperListJSONAndHelperNextSpecificID(t *testing.T) {
	b := newBus(t)
	mustWith(t, b, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, Status: HelperQueued, Request: "req1"}
		st.Helpers["2"] = &Helper{ID: 2, Status: HelperQueued, Request: "req2"}
	})
	// Take specific helper ID 2
	mustWith(t, b, func(st *State) {
		h := takeLocalHelper(st, "worker-1", nil, 2)
		if h == nil || h.ID != 2 || h.Status != HelperStarting || h.Worker != "worker-1" {
			t.Fatalf("expected helper 2 to be taken, got: %+v", h)
		}
	})
}

func TestTakeLocalHelperRefusesAlreadyServingWorker(t *testing.T) {
	b := newBus(t)
	mustWith(t, b, func(st *State) {
		st.Helpers["1"] = &Helper{ID: 1, Status: HelperStarting, Worker: "worker-1", Request: "req1"}
		st.Helpers["2"] = &Helper{ID: 2, Status: HelperQueued, Request: "req2"}
	})
	mustWith(t, b, func(st *State) {
		h := takeLocalHelper(st, "worker-1", nil, 2)
		if h != nil {
			t.Fatalf("expected nil when worker-1 is already starting helper 1, got %+v", h)
		}
	})
	mustWith(t, b, func(st *State) {
		st.Helpers["1"].Status = HelperRunning
		h := takeLocalHelper(st, "worker-1", nil, 2)
		if h != nil {
			t.Fatalf("expected nil when worker-1 is already running helper 1, got %+v", h)
		}
	})
}

func TestFreePoolWorkersObservationCauses(t *testing.T) {
	b := newBus(t)
	// 1. Worker in active legacy crew
	mustWith(t, b, func(st *State) {
		st.Teams["team-1"] = &Team{
			ID:   "team-1",
			Done: false,
			Members: map[string]*TeamMember{
				"w1": {Agent: "w1", Tier: TierWorker, State: MemberSleeping},
			},
		}
		if free := freePoolWorkersState(st, []string{"w1"}); free != 0 {
			t.Fatalf("expected 0 free workers when w1 is in active legacy crew, got %d", free)
		}
	})

	// 2. Worker in active ordinary/manual run
	mustWith(t, b, func(st *State) {
		delete(st.Teams, "team-1")
		st.Teams["team-2"] = &Team{
			ID:   "team-2",
			Done: false,
			Members: map[string]*TeamMember{
				"w1": {Agent: "w1", Tier: TierNormal, State: MemberActive},
			},
		}
		if free := freePoolWorkersState(st, []string{"w1"}); free != 0 {
			t.Fatalf("expected 0 free workers when w1 is active in team-2, got %d", free)
		}
	})

	// 3. Worker with unsettled workspace record in tasksDir
	tasksDir := t.TempDir()
	taskPath := filepath.Join(tasksDir, "task-requester")
	if err := os.MkdirAll(taskPath, 0o750); err != nil {
		t.Fatal(err)
	}
	recordPath := filepath.Join(taskPath, "request-workspace-1.json")
	if err := os.WriteFile(recordPath, []byte(`{"worker":"w1","stage":"working"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	mustWith(t, b, func(st *State) {
		delete(st.Teams, "team-2")
		if free := freePoolWorkersState(st, []string{"w1"}, tasksDir); free != 0 {
			t.Fatalf("expected 0 free workers when w1 has unsettled workspace, got %d", free)
		}
	})

	// 4. Truly free worker
	mustWith(t, b, func(st *State) {
		_ = os.Remove(recordPath)
		if free := freePoolWorkersState(st, []string{"w1"}, tasksDir); free != 1 {
			t.Fatalf("expected 1 free worker when w1 is clear, got %d", free)
		}
	})
}
