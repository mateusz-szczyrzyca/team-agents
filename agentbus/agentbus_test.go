package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// ---------------------------------------------------------------- helpers

// newBus returns a bus in a fresh temporary state directory.
func newBus(t *testing.T) *Bus {
	t.Helper()
	return &Bus{dir: t.TempDir()}
}

// server creates an MCP server instance for agent name on bus b (no stdio involved).
func server(b *Bus, name string, opts ...func(*Server)) *Server {
	s := &Server{
		bus: b, me: name, out: json.NewEncoder(io.Discard), role: RoleAgent, queueMax: 3,
		tier: TierNormal, tasksDir: filepath.Join(b.dir, "tasks"),
		info: Presence{Agent: name, Task: "task-" + name, Repo: "repo", Branch: "agents/" + name, Worktree: "/tmp/wt-" + name},
	}
	for _, o := range opts {
		o(s)
	}
	return s
}

func inTeam(id string) func(*Server) { return func(s *Server) { s.team = id } } //nolint:unparam // The shared team ID T is intentional fixture data; retain the reusable helper.
func asWorker(s *Server)             { s.tier = "worker" }

// createTeam stores a team with the given members (name → tier/provider/state).
func createTeam(t *testing.T, b *Bus, id string, members map[string][3]string) { //nolint:unparam // The shared team ID T is intentional fixture data; retain the reusable helper.
	t.Helper()
	team := &Team{ID: id, Repo: "repo", Members: map[string]*TeamMember{}}
	for name, v := range members {
		team.Members[name] = &TeamMember{Agent: name, Task: "task-" + name, Tier: Tier(v[0]), Provider: v[1], State: MemberState(v[2])}
	}
	mustWith(t, b, func(st *State) { st.Teams[id] = team })
}

// mustWith runs fn under the bus lock and fails the test on a storage error.
func mustWith(t *testing.T, b *Bus, fn func(*State)) {
	t.Helper()
	if serr := b.locked(fn); serr != nil {
		t.Fatal(serr)
	}
}

// captureWakes replaces wakeFn for the duration of a test and records woken task IDs.
func captureWakes(t *testing.T) *[]string {
	t.Helper()
	var mu sync.Mutex
	woken := &[]string{}
	old := wakeFn
	wakeFn = func(_ *Team, task string) { mu.Lock(); *woken = append(*woken, task); mu.Unlock() }
	t.Cleanup(func() { wakeFn = old })
	return woken
}

// check wraps *testing.T for successful calls. Tool handlers return their own error types; wrap them in
// toolResult, which turns a nil *OpError into a nil error:
//
//	chk(t).ok(toolResult(s.send(...)))                          // must succeed
//	failsWith(t, BoardClaimWorkerNeedsEasy)(w.boardClaim("B2"))  // must fail with exactly this cause
type check struct{ t *testing.T }

// chk returns a check helper for t.
func chk(t *testing.T) check { return check{t} }

// ok fails the test on an error and returns the output.
func (c check) ok(out string, err error) string {
	c.t.Helper()
	if err != nil {
		c.t.Fatalf("unexpected error: %v", err)
	}
	return out
}

// failsWith returns a function asserting that a call failed with exactly the cause want. It also checks
// that the class sentinel matches with errors.Is and that errors.As finds the same error in a plain error value.
func failsWith[C opCause](t *testing.T, want C) func(string, *OpError[C]) {
	return func(_ string, err *OpError[C]) {
		t.Helper()
		expectCause(t, err, want)
	}
}

// failure is failsWith for tests that examine the error further: it returns the checked error.
func failure[C opCause](t *testing.T, want C) func(string, *OpError[C]) *OpError[C] {
	return func(_ string, err *OpError[C]) *OpError[C] {
		t.Helper()
		expectCause(t, err, want)
		return err
	}
}

// expectCause asserts that err is an *OpError[C] with the cause want (see failsWith).
func expectCause[C opCause](t *testing.T, err *OpError[C], want C) {
	t.Helper()
	if err == nil {
		t.Fatalf("expected %s: %s, got no error", want.op(), want.info().name)
	}
	if err.Cause != want {
		t.Fatalf("cause = %s, want %s (%v)", err.Cause.info().name, want.info().name, err)
	}
	if !errors.Is(err, want.info().class.Sentinel()) {
		t.Fatalf("errors.Is(%q, %v) = false", err, want.info().class.Sentinel())
	}
	var target *OpError[C]
	if !errors.As(err.asError(), &target) || target != err {
		t.Fatalf("errors.As does not find the %T", err)
	}
	if err.Message == "" {
		t.Fatalf("error without a message: %#v", err)
	}
}

// expectToolError asserts that a plain error from the dispatcher (Server.call) is an *OpError[C] with the
// cause want.
func expectToolError[C opCause](t *testing.T, err error, want C) {
	t.Helper()
	expectCause(t, opErrorOf[C](t, err), want)
}

// toolFailure is expectToolError for tests that examine the error further: it returns the checked error.
func toolFailure[C opCause](t *testing.T, err error, want C) *OpError[C] {
	t.Helper()
	e := opErrorOf[C](t, err)
	expectCause(t, e, want)
	return e
}

// opErrorOf extracts the *OpError[C] from a plain error value (the test fails when there is none).
func opErrorOf[C opCause](t *testing.T, err error) *OpError[C] {
	t.Helper()
	var e *OpError[C]
	if !errors.As(err, &e) {
		t.Fatalf("errors.As(%v) found no %T", err, e)
	}
	return e
}

func allMessages(t *testing.T, b *Bus) string {
	t.Helper()
	msgs, err := b.messages()
	if err != nil {
		t.Fatal(err)
	}
	var sb strings.Builder
	for _, m := range msgs {
		sb.WriteString(fmtMsg(m) + "\n")
	}
	return sb.String()
}

// ---------------------------------------------------------------- messages

func TestSendAndRead(t *testing.T) {
	b := newBus(t)
	a, c := server(b, "a"), server(b, "c")
	chk(t).ok(toolResult(a.send("c", "hello c")))
	chk(t).ok(toolResult(a.send("all", "hello all")))
	out := chk(t).ok(toolResult(c.read(0)))
	if !strings.Contains(out, "hello c") || !strings.Contains(out, "hello all") {
		t.Fatalf("c should see both messages, got:\n%s", out)
	}
	if out := chk(t).ok(toolResult(c.read(0))); !strings.Contains(out, "No new messages") {
		t.Fatalf("second read should be empty, got:\n%s", out)
	}
	failsWith(t, SendMessageToSelf)(a.send("a", "to myself"))
}

// ---------------------------------------------------------------- reservations

func TestReserveRelease(t *testing.T) {
	b := newBus(t)
	a, c := server(b, "a"), server(b, "c")
	if out := chk(t).ok(toolResult(a.reserve("probe-1", "flash", 0))); !strings.HasPrefix(out, "OK") {
		t.Fatalf("reserve: %s", out)
	}
	if out := chk(t).ok(toolResult(c.reserve("probe-1", "", 0))); !strings.HasPrefix(out, "TAKEN") {
		t.Fatalf("second reserve should report TAKEN, got: %s", out)
	}
	failsWith(t, ReleaseNotHolder)(c.release("probe-1"))
	chk(t).ok(toolResult(a.release("probe-1")))
	if out := chk(t).ok(toolResult(c.reserve("probe-1", "", 0))); !strings.HasPrefix(out, "OK") {
		t.Fatalf("reserve after release: %s", out)
	}
}

// ---------------------------------------------------------------- investigation queue

func requestN(t *testing.T, b *Bus, n int) []string {
	t.Helper()
	var replies []string
	for i := 0; i < n; i++ {
		s := server(b, string(rune('a'+i)))
		out, err := s.requestInvestigation("question", "", false)
		if err != nil {
			t.Fatal(err)
		}
		replies = append(replies, strings.SplitN(out, ":", 2)[0])
	}
	return replies
}

func TestQueueCapacityOneInvestigator(t *testing.T) {
	old := invSlots
	invSlots = 1
	defer func() { invSlots = old }()
	got := requestN(t, newBus(t), 6)
	want := []string{"ACCEPTED", "ACCEPTED", "ACCEPTED", "ACCEPTED", "QUEUE FULL", "QUEUE FULL"}
	if strings.Join(got, ",") != strings.Join(want, ",") {
		t.Fatalf("got %v, want %v", got, want)
	}
}

func TestQueueCapacityTwoInvestigators(t *testing.T) {
	old := invSlots
	invSlots = 2
	defer func() { invSlots = old }()
	got := requestN(t, newBus(t), 6)
	accepted := 0
	for _, r := range got {
		if r == "ACCEPTED" {
			accepted++
		}
	}
	if accepted != 5 {
		t.Fatalf("2 investigators + 3 waiting should accept 5, got %v", got)
	}
}

func TestWaitroomAndDecision(t *testing.T) {
	old := invSlots
	invSlots = 1
	defer func() { invSlots = old }()
	b := newBus(t)
	requestN(t, b, 4)
	e := server(b, "e")
	out, err := e.requestInvestigation("q", "", true)
	if err != nil || !strings.HasPrefix(out, "WAITING ROOM") {
		t.Fatalf("expected WAITING ROOM, got %q (%v)", out, err)
	}
	// a rejected request (queue full, no wait) must be resolved with record_decision
	f := server(b, "f")
	out = chk(t).ok(toolResult(f.requestInvestigation("q", "", false)))
	if !strings.HasPrefix(out, "QUEUE FULL") {
		t.Fatalf("expected QUEUE FULL, got %q", out)
	}
	var id int64
	mustWith(t, b, func(st *State) {
		for _, inv := range st.Investigations {
			if inv.FromAgent == "f" {
				id = inv.ID
			}
		}
	})
	chk(t).ok(toolResult(f.recordDecision(int(id), "solve it myself", "small problem")))
}

// ---------------------------------------------------------------- teams: coordinator

func TestClaimLeadTiers(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{
		"n1": {"normal", "anthropic", "active"}, "n2": {"normal", "openai", "active"}, "w1": {"worker", "openrouter", "active"},
	})
	w1 := server(b, "w1", inTeam("T"), asWorker)
	if out := chk(t).ok(toolResult(w1.claimLead())); !strings.Contains(out, "worker") {
		t.Fatalf("a worker must not become coordinator: %s", out)
	}
	n1, n2 := server(b, "n1", inTeam("T")), server(b, "n2", inTeam("T"))
	if out := chk(t).ok(toolResult(n1.claimLead())); !strings.Contains(out, "You are the coordinator") {
		t.Fatalf("n1 should win: %s", out)
	}
	if out := chk(t).ok(toolResult(n2.claimLead())); !strings.Contains(out, "n1") {
		t.Fatalf("n2 should learn that n1 coordinates: %s", out)
	}
	failsWith(t, HandoverLeadTargetIsWorker)(n1.handoverLead("w1"))
}

func TestHungCoordinatorCanBeReplaced(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"n1": {"normal", "a", "active"}, "n2": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "n1", inTeam("T")).claimLead()))
	mustWith(t, b, func(st *State) { st.Teams["T"].Members["n1"].State = MemberHung })
	if out := chk(t).ok(toolResult(server(b, "n2", inTeam("T")).claimLead())); !strings.Contains(out, "taken over") {
		t.Fatalf("n2 should take over from the hung coordinator: %s", out)
	}
}

// ---------------------------------------------------------------- teams: quiz

func TestQuizAnswersArePrivateAndFinal(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	failsWith(t, QuizStartNotCoordinator)(m.quizStart("only the coordinator asks"))
	chk(t).ok(toolResult(lead.quizStart("How many bits in a byte? Number only.")))
	failsWith(t, QuizAnswerCoordinatorAnswers)(lead.quizAnswer("8"))
	chk(t).ok(toolResult(m.quizAnswer("SECRET-8")))
	failsWith(t, QuizAnswerAlreadyAnswered)(m.quizAnswer("9"))
	if strings.Contains(allMessages(t, b), "SECRET-8") {
		t.Fatal("quiz answers must never appear on the message log")
	}
	if out := chk(t).ok(toolResult(lead.quizResults(false))); !strings.Contains(out, "SECRET-8") {
		t.Fatalf("the coordinator must see the answer: %s", out)
	}
	chk(t).ok(toolResult(lead.teamExclude("m", "wrong answer")))
	mustWith(t, b, func(st *State) {
		if st.Teams["T"].Members["m"].State != MemberExcluded {
			t.Fatal("member should be excluded")
		}
	})
}

// ---------------------------------------------------------------- teams: board

func TestBoardTiersAndResults(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "w": {"worker", "b", "active"}})
	lead, w := server(b, "lead", inTeam("T")), server(b, "w", inTeam("T"), asWorker)
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.boardAdd("count files", "ls | wc -l", "easy")))
	chk(t).ok(toolResult(lead.boardAdd("refactor", "hard work", "normal")))
	failsWith(t, BoardClaimWorkerNeedsEasy)(w.boardClaim("B2")) // workers take easy items only
	chk(t).ok(toolResult(w.boardClaim("B1")))
	failsWith(t, BoardClaimAlreadyTaken)(lead.boardClaim("B1")) // already claimed
	chk(t).ok(toolResult(w.boardDone("B1", "42 files")))
	if out := chk(t).ok(toolResult(lead.read(0))); !strings.Contains(out, "RESULT B1") || !strings.Contains(out, "42 files") {
		t.Fatalf("the creator should receive the result, got:\n%s", out)
	}
}

func TestWakeOneBalancesProviders(t *testing.T) {
	b := newBus(t)
	woken := captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{
		"lead": {"normal", "openrouter", "active"},
		"wa":   {"worker", "openrouter", "sleeping"},
		"wb":   {"worker", "deepinfra", "sleeping"},
	})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	// claim_lead announces the coordinator to the team, which wakes sleeping members; restore them
	*woken = nil
	mustWith(t, b, func(st *State) {
		for _, n := range []string{"wa", "wb"} {
			st.Teams["T"].Members[n].State = MemberSleeping
		}
	})
	chk(t).ok(toolResult(lead.boardAdd("easy thing", "details", "easy")))
	if len(*woken) != 1 || (*woken)[0] != "task-wb" {
		t.Fatalf("the worker of the less busy provider (deepinfra) should be woken, got %v", *woken)
	}
}

// ---------------------------------------------------------------- worker requests

func TestDelegateTierRules(t *testing.T) {
	b := newBus(t)
	var spawned []string
	old := agentCmdFn
	agentCmdFn = func(_, _ string, args ...string) { spawned = append(spawned, strings.Join(args, " ")) }
	defer func() { agentCmdFn = old }()
	failsWith(t, DelegateNotNormalTier)(server(b, "w", asWorker).delegate("x", ""))
	chk(t).ok(toolResult(server(b, "n").delegate("count files", "src/")))
	if len(spawned) != 1 || spawned[0] != "_dispatch-helpers" {
		t.Fatalf("delegate should spawn the dispatcher once, got %v", spawned)
	}
}

// ---------------------------------------------------------------- checkpoints

func TestCheckpointIncludesBoard(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.claimLead()))
	chk(t).ok(toolResult(lead.boardAdd("chapter 2", "write it", "normal")))
	chk(t).ok(toolResult(lead.checkpoint("chapter 1 done", "chapter 2 next")))
	data, err := os.ReadFile(filepath.Join(b.dir, "checkpoints", "team-T.md"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"chapter 1 done", "chapter 2 next", "B1"} {
		if !bytes.Contains(data, []byte(want)) {
			t.Fatalf("checkpoint should contain %q:\n%s", want, data)
		}
	}
}

// ---------------------------------------------------------------- normalize

func normalizeLines(t *testing.T, typ string, events ...string) []activity {
	t.Helper()
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	n := &normalizer{enc: enc, deltas: map[float64]*strings.Builder{}}
	for _, e := range events {
		var ev map[string]any
		if err := json.Unmarshal([]byte(e), &ev); err != nil {
			t.Fatal(err)
		}
		switch typ {
		case "claude":
			n.claude(ev)
		case "codex":
			n.codex(ev)
		case "opencode":
			n.opencode(ev)
		}
	}
	var out []activity
	dec := json.NewDecoder(&buf)
	for dec.More() {
		var a activity
		if err := dec.Decode(&a); err != nil {
			t.Fatal(err)
		}
		out = append(out, a)
	}
	return out
}

func TestNormalizeClaude(t *testing.T) {
	got := normalizeLines(t, "claude",
		`{"type":"assistant","message":{"content":[{"type":"text","text":"Looking at the code"},{"type":"tool_use","name":"Bash","input":{"command":"cargo build"}}]}}`,
		`{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__agentbus__reserve","input":{"resource":"probe-1"}}]}}`)
	kinds := []string{}
	for _, a := range got {
		kinds = append(kinds, string(a.Kind))
	}
	if strings.Join(kinds, ",") != "text,cmd,mcp" {
		t.Fatalf("unexpected kinds %v (%+v)", kinds, got)
	}
}

func TestNormalizeOpenCode2ToolNames(t *testing.T) {
	got := normalizeLines(t, "opencode",
		`{"type":"tool_use","part":{"tool":"execute","state":{"status":"completed","input":{"command":"ls"}}}}`,
		`{"type":"tool_use","part":{"tool":"agentbus.list_agents","state":{"status":"completed","input":{}}}}`,
		`{"type":"text","part":{"text":"STATUS: DONE"}}`)
	if len(got) != 3 || got[0].Kind != "cmd" || got[1].Kind != "mcp" || got[2].Kind != "text" {
		t.Fatalf("unexpected normalization: %+v", got)
	}
}

func TestTrunc(t *testing.T) {
	if got := trunc("a\n  b\tc", 10); got != "a b c" {
		t.Fatalf("whitespace should collapse, got %q", got)
	}
	if got := trunc(strings.Repeat("x", 20), 5); got != "xxxxx…" {
		t.Fatalf("should keep 5 runes and add an ellipsis, got %q", got)
	}
}
