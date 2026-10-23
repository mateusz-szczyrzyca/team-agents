package main

import (
	"bufio"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestMain makes every test that calls the `agent` script without a stub fail loudly: no test may run the real
// installation (a lesson from tests that woke real tasks).
func TestMain(m *testing.M) {
	callAgent = func(agentCmd string, args ...string) (string, *OpError[AgentCallCause]) {
		panic(fmt.Sprintf("callAgent(%s %v) without a stub — a test would have run the real agent script", agentCmd, args))
	}
	callTailscale = func(cli, ip string) (*tsWhois, *OpError[TailscaleCause]) {
		panic(fmt.Sprintf("callTailscale(%s, %s) without a stub — a test would have asked the real tailscaled", cli, ip))
	}
	os.Exit(m.Run())
}

// stubAgent replaces callAgent for one test.
func stubAgent(t *testing.T, fn func(args ...string) (string, *OpError[AgentCallCause])) {
	t.Helper()
	prev := callAgent
	callAgent = func(_ string, args ...string) (string, *OpError[AgentCallCause]) { return fn(args...) }
	t.Cleanup(func() { callAgent = prev })
}

// writeFile writes a fixture file.
func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o750); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
}

// questionsFixture is a questions file with a recommendation and options (1), an old-style label without a
// recommendation (2), and the answering instructions at the top.
const questionsFixture = `# Questions — T2 (round 1)

> Answer every question after **Verdict:** — ` + "`ok`" + ` accepts the recommendation. When you are done, put **ANSWERED** on the last line and save: the answers are sent at once (or run ` + "`agent resume T2`" + `).

## 1. Where should the cache live?
a) in memory — lost on restart
b) on disk — survives restarts

**Recommended:** a) — nothing to clean up (lead)
Other views: member preferred b

**Verdict:**

## 2. Which name?

**Answer:**
`

// fixture builds an AGENTS_HOME: a running solo task (T1), a task waiting for answers (T2), a finished one (T3),
// a team (TT) with a coordinator (T4) and a member (T5), the bus state, messages and a log.
func fixture(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	task := func(id, meta string) string {
		dir := filepath.Join(home, "tasks", id)
		writeFile(t, filepath.Join(dir, "meta.env"), "ID="+id+"\nTYPE=claude\nREPO_NAME=repo\nBRANCH=agents/x\nCREATED=2026-10-23T08:00:00+02:00\n"+meta)
		return dir
	}
	d1 := task("T1", "AGENT=solo\nSTATUS=running\nRUN=2\n")
	writeFile(t, filepath.Join(d1, "activity.jsonl"), `{"ts":"2026-10-23 08:01:00","k":"cmd","t":"make test"}`+"\n"+`{"ts":"2026-10-23 08:02:00","k":"text","t":"tests pass"}`+"\n")
	writeFile(t, filepath.Join(d1, "prompt-1.md"), "first prompt")
	writeFile(t, filepath.Join(d1, "prompt-2.md"), "SECOND PROMPT")
	writeFile(t, filepath.Join(d1, "err-2.log"), "boom on stderr")
	d2 := task("T2", "AGENT=asker\nSTATUS=waiting\nRUN=1\nROUND=1\n")
	writeFile(t, filepath.Join(d2, "questions-1.md"), questionsFixture)
	d3 := task("T3", "AGENT=finisher\nSTATUS=done\nRUN=1\nFINISHED=2026-10-23T09:00:00+02:00\n")
	writeFile(t, filepath.Join(d3, "final.md"), "All done.\n\nSTATUS: DONE")
	task("T4", "AGENT=lead\nSTATUS=running\nRUN=1\nTEAM=TT-team\n")
	task("T5", "AGENT=member\nSTATUS=paused\nRUN=3\nTEAM=TT-team\nWAKE_AT=1792735200\nPAUSE_REASON=quota\n")
	writeFile(t, filepath.Join(home, "bus", "state.json"), `{
 "agents": {"solo": {"agent": "solo", "task": "T1", "status": "running the tests"}},
 "reservations": {"probe": {"resource": "probe", "agent": "solo", "task": "T1", "note": "flashing", "since": "2026-10-23 08:05:00"}},
 "investigations": {"7": {"id": 7, "from_agent": "solo", "from_task": "T1", "question": "Is DMA safe here?", "status": "done",
   "summary": "yes, with a fence", "kind": "review", "pr_url": "https://github.com/o/r/pull/4", "file": "`+filepath.Join(home, "inv-7.md")+`"}},
 "teams": {"TT-team": {"id": "TT-team", "lead": "lead", "done": false,
   "members": {"lead": {"state": "active", "tier": "normal", "task": "T4"}, "member": {"state": "paused", "tier": "normal", "task": "T5"}},
   "board": {"B1": {"id": "B1", "title": "Write the parser", "details": "parse it", "level": "normal", "status": "todo", "creator": "lead", "for": "member"},
             "B2": {"id": "B2", "title": "List files", "details": "ls", "level": "easy", "status": "done", "creator": "lead", "assignee": "member", "result": "3 files"}},
   "consults": {"C1": {"id": "C1", "from": "lead", "text": "1. a or b?", "asked": ["member"], "started": "2026-10-23 08:00:00",
     "deadline": "2026-10-23 08:05:00", "replies": {}}}}}
}`)
	writeFile(t, filepath.Join(home, "inv-7.md"), "# Investigation 7\nthe record")
	writeFile(t, filepath.Join(home, "bus", "messages.jsonl"),
		`{"id":1,"ts":"2026-10-23 08:00:00","from":"lead","to":"team:TT-team","kind":"chat","text":"hello team"}`+"\n"+
			`{"id":2,"ts":"2026-10-23 08:01:00","from":"solo","to":"all","kind":"checkpoint","text":"CHECKPOINT: solo step"}`+"\n")
	writeFile(t, filepath.Join(home, "logs", "agent.jsonl"),
		`{"ts":"2026-10-23T06:00:00Z","level":"info","component":"agent","event":"run_started","task":"T1"}`+"\n"+
			`{"ts":"2026-10-23T06:01:00Z","level":"warn","component":"agent","event":"notification_failed","task":"T1","exit_code":"127"}`+"\n")
	return home
}

// testServer serves the web interface for a fixture home.
func testServer(t *testing.T, home string) (*webServer, *httptest.Server) {
	t.Helper()
	cfg := webConfig{config: config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", queueMax: 3},
		users: map[string]bool{"me@example.com": true}, poll: 30 * time.Millisecond}
	s, serr := newWebServer(cfg)
	if serr != nil {
		t.Fatal(serr)
	}
	ts := httptest.NewServer(s.handler())
	t.Cleanup(ts.Close)
	return s, ts
}

// get fetches a path and returns the status and the body.
func get(t *testing.T, ts *httptest.Server, path string, hdr ...string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, ts.URL+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i+1 < len(hdr); i += 2 {
		req.Header.Set(hdr[i], hdr[i+1])
	}
	resp, err := ts.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer closeBody(t, resp)
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	return resp.StatusCode, string(b)
}

// closeBody closes a response body (a failure is a test failure).
func closeBody(t *testing.T, resp *http.Response) {
	t.Helper()
	if err := resp.Body.Close(); err != nil {
		t.Errorf("closing the response body: %v", err)
	}
}

// post submits the answer form as htmx does (HX-Request).
func post(t *testing.T, ts *httptest.Server, path string, form url.Values) (int, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodPost, ts.URL+path, strings.NewReader(form.Encode()))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("HX-Request", "true")
	req.Header.Set("Origin", ts.URL)
	resp, err := ts.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer closeBody(t, resp)
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	return resp.StatusCode, string(b)
}

// mustContain fails when the body lacks any of the strings.
func mustContain(t *testing.T, where, body string, want ...string) {
	t.Helper()
	for _, w := range want {
		if !strings.Contains(body, w) {
			t.Errorf("%s: %q missing", where, w)
		}
	}
}

func TestEveryViewRenders(t *testing.T) {
	home := fixture(t)
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		if args[0] != "_agents-json" {
			t.Fatalf("unexpected agent call %v", args)
		}
		return `{"name":"solo","type":"claude","model":"opus","tier":"normal","provider":"anthropic"}` + "\n" +
			`{"name":"member","type":"codex","model":"","tier":"normal","provider":"openai"}` + "\n", nil
	})
	_, ts := testServer(t, home)
	cases := []struct {
		path string
		want []string
	}{
		{"/", []string{"Dashboard", ">solo<", "running the tests", "? waiting for you", "probe", "flashing", `sse-connect="/events/dashboard"`, "team TT"}},
		{"/agents", []string{"solo", "opus", "anthropic", "running the tests", "member", "(quota exhausted)"}},
		{"/task/T1", []string{"make test", "tests pass", `sse-connect="/events/task/T1?offset=`}},
		{"/task/T1?tab=prompt", []string{"SECOND PROMPT", "first prompt", "current"}},
		{"/task/T1?tab=errors", []string{"boom on stderr"}},
		{"/task/T1?tab=investigations", []string{"Is DMA safe here?", "yes, with a fence", "pull/4"}},
		{"/task/T1?tab=meta", []string{"REPO_NAME", "agents/x"}},
		{"/task/T3?tab=report", []string{"All done."}},
		{"/task/T2?tab=questions", []string{"Where should the cache live?", "round 1"}},
		{"/task/T2", []string{`href="/questions/T2"`}},
		{"/teams", []string{"TT-team", "lead"}},
		{"/team/TT-team", []string{"Write the parser", "for member", "3 files", "C1", "NO ANSWER", "hello team", "paused"}},
		{"/bus", []string{"hello team", "CHECKPOINT: solo step", `sse-connect="/events/bus?offset=`}},
		{"/bus?kind=checkpoint", []string{"CHECKPOINT: solo step"}},
		{"/questions", []string{"asker", "2 questions", `href="/questions/T2"`}},
		{"/questions/T2", []string{"Where should the cache live?", "nothing to clean up", `value="ok" checked`, "in memory", "Which name?"}},
		{"/investigations", []string{"#7", "review", "yes, with a fence"}},
		{"/investigation/7", []string{"the record"}},
		{"/logs", []string{"run_started", "notification_failed"}},
		{"/logs?level=warn", []string{"notification_failed"}},
		{"/healthz", []string{"ok"}},
	}
	for _, c := range cases {
		code, body := get(t, ts, c.path)
		if code != http.StatusOK {
			t.Errorf("%s: status %d\n%s", c.path, code, body)
			continue
		}
		mustContain(t, c.path, body, c.want...)
	}
	// filters really filter
	if _, body := get(t, ts, "/bus?kind=checkpoint"); strings.Contains(body, "hello team") {
		t.Error("/bus?kind=checkpoint shows a chat message")
	}
	if _, body := get(t, ts, "/logs?level=warn"); strings.Contains(body, "run_started") {
		t.Error("/logs?level=warn shows an info record")
	}
}

func TestMissingThingsAndBadParameters(t *testing.T) {
	_, ts := testServer(t, fixture(t))
	for path, want := range map[string]int{
		"/task/nope": http.StatusNotFound, "/team/nope": http.StatusNotFound, "/investigation/99": http.StatusNotFound,
		"/task/T1?tab=bogus": http.StatusBadRequest, "/logs?src=../../etc/passwd": http.StatusBadRequest,
		"/questions/T1": http.StatusConflict, "/questions/nope": http.StatusNotFound,
	} {
		if code, body := get(t, ts, path); code != want {
			t.Errorf("%s: status %d, want %d\n%s", path, code, want, body)
		}
	}
}

func TestAccessIsRefusedForEveryCause(t *testing.T) {
	s, _ := testServer(t, fixture(t))
	req := func(method, remote string, hdr ...string) *http.Request {
		r := httptest.NewRequest(method, "http://127.0.0.1:8787/", nil)
		r.RemoteAddr = remote
		for i := 0; i+1 < len(hdr); i += 2 {
			r.Header.Set(hdr[i], hdr[i+1])
		}
		return r
	}
	cases := []struct {
		name string
		r    *http.Request
		want AccessCause
		ok   bool
	}{
		{"local browser", req("GET", "127.0.0.1:5555"), 0, true},
		{"allowed Tailscale user", req("GET", "127.0.0.1:5555", "Tailscale-User-Login", "Me@Example.com"), 0, true},
		{"remote connection", req("GET", "192.0.2.7:5555"), AccessNotLoopback, false},
		{"unknown Tailscale user", req("GET", "127.0.0.1:5555", "Tailscale-User-Login", "eve@example.com"), AccessUnknownUser, false},
		{"Funnel (proxied, no identity)", req("GET", "127.0.0.1:5555", "X-Forwarded-For", "203.0.113.9"), AccessProxiedAnonymous, false},
		{"Funnel header", req("GET", "127.0.0.1:5555", "Tailscale-Funnel-Request", "?1"), AccessProxiedAnonymous, false},
		{"POST without htmx", req("POST", "127.0.0.1:5555"), AccessNotHTMX, false},
		{"POST from another site", req("POST", "127.0.0.1:5555", "HX-Request", "true", "Sec-Fetch-Site", "cross-site"), AccessCrossSite, false},
		{"POST from another origin", req("POST", "127.0.0.1:5555", "HX-Request", "true", "Origin", "https://evil.example"), AccessCrossSite, false},
		{"POST from this page", req("POST", "127.0.0.1:5555", "HX-Request", "true", "Origin", "http://127.0.0.1:8787", "Sec-Fetch-Site", "same-origin"), 0, true},
	}
	for _, c := range cases {
		e := s.checkAccess(c.r)
		if c.ok {
			if e != nil {
				t.Errorf("%s: refused: %v", c.name, e)
			}
			continue
		}
		var oe *OpError[AccessCause]
		if err := error(e); e == nil || !errors.As(err, &oe) || oe.Cause != c.want || !errors.Is(err, ErrWeb) {
			t.Errorf("%s: got %v, want cause %v", c.name, e, c.want)
		}
	}
	// and through the handler: 403 with the security headers
	rec := httptest.NewRecorder()
	s.handler().ServeHTTP(rec, req("GET", "192.0.2.7:5555"))
	if rec.Code != http.StatusForbidden || !strings.Contains(rec.Header().Get("Content-Security-Policy"), "default-src 'self'") {
		t.Errorf("handler: %d, CSP %q", rec.Code, rec.Header().Get("Content-Security-Policy"))
	}
}

func TestListensOnLoopbackOnly(t *testing.T) {
	for addr, want := range map[string]StartCause{"0.0.0.0:0": StartNotLoopback, "192.0.2.1:8787": StartNotLoopback, "nonsense": StartBadListen} {
		ln, e := listenLoopback(addr)
		if ln != nil {
			t.Errorf("%s: listening", addr)
			if err := ln.Close(); err != nil {
				t.Error(err)
			}
		}
		var oe *OpError[StartCause]
		if err := error(e); e == nil || !errors.As(err, &oe) || oe.Cause != want {
			t.Errorf("%s: got %v, want %v", addr, e, want)
		}
	}
	ln, e := listenLoopback("127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	if err := ln.Close(); err != nil {
		t.Error(err)
	}
}

func TestQuestionsFileParseAndRender(t *testing.T) {
	d := parseQuestions(questionsFixture)
	if len(d.Questions) != 2 || d.Answered {
		t.Fatalf("parsed %d questions, answered=%v", len(d.Questions), d.Answered)
	}
	q1, q2 := d.Questions[0], d.Questions[1]
	if q1.Num != "1" || q1.Title != "Where should the cache live?" || len(q1.Options) != 2 || q1.Options[1].Letter != "b" ||
		!strings.Contains(q1.Recommended, "nothing to clean up") || !strings.Contains(q1.Recommended, "Other views") || q1.Verdict != "" {
		t.Errorf("question 1: %+v", q1)
	}
	if q2.Recommended != "" || q2.verdict < 0 {
		t.Errorf("question 2 (old label, no recommendation): %+v", q2)
	}
	out := d.render(map[int]string{0: "ok\nkeep it small", 1: "parser"}, true)
	for _, want := range []string{"**Verdict:** ok\nkeep it small\n", "**Verdict:** parser\n", "> Answer every question after", "**Recommended:** a) — nothing to clean up (lead)"} {
		if !strings.Contains(out, want) {
			t.Errorf("rendered file lacks %q:\n%s", want, out)
		}
	}
	if !strings.HasSuffix(out, "\n**ANSWERED**\n") || strings.Contains(out, "**Answer:**") {
		t.Errorf("marker or label:\n%s", out)
	}
	// the result parses back: verdicts kept, marked, still two questions
	back := parseQuestions(out)
	if !back.Answered || len(back.Questions) != 2 || back.Questions[0].Verdict != "ok\nkeep it small" || back.Questions[1].Verdict != "parser" {
		t.Errorf("round trip: %+v / %+v", back.Questions[0], back.Questions[1])
	}
	// a draft (no marker) removes an earlier marker
	if parseQuestions(back.render(map[int]string{1: "other"}, false)).Answered {
		t.Error("a draft keeps the ANSWERED marker")
	}
}

func TestComposeAndSplitVerdicts(t *testing.T) {
	q := parseQuestions(questionsFixture).Questions[0]
	for _, c := range []struct{ choice, text, want string }{
		{"ok", "", "ok"}, {"ok", "but fast", "ok\nbut fast"}, {"b", "", "b) on disk — survives restarts"},
		{"own", "neither", "neither"}, {"own", "", ""}, {"", "", ""},
	} {
		if got := composeVerdict(q, c.choice, c.text); got != c.want {
			t.Errorf("compose(%q, %q) = %q, want %q", c.choice, c.text, got, c.want)
		}
	}
	q.Verdict = "b) on disk — survives restarts\nplus a note"
	if c, txt := splitVerdict(q); c != "b" || txt != "plus a note" {
		t.Errorf("split: %q %q", c, txt)
	}
	q.Verdict = ""
	if c, _ := splitVerdict(q); c != choiceOK {
		t.Errorf("an empty verdict with a recommendation pre-selects ok, got %q", c)
	}
}

func TestAnswerFormDraftIncompleteAndSend(t *testing.T) {
	home := fixture(t)
	_, ts := testServer(t, home)
	qfile := filepath.Join(home, "tasks", "T2", "questions-1.md")
	before, err := os.Stat(qfile)
	if err != nil {
		t.Fatal(err)
	}
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		t.Fatalf("agent called for a draft or incomplete answers: %v", args)
		return "", nil
	})
	// a draft: verdicts saved, no marker, nothing sent
	code, body := post(t, ts, "/questions/T2", url.Values{"c0": {"b"}, "t0": {""}, "c1": {"own"}, "t1": {""}, "action": {"draft"}})
	mustContain(t, "draft", body, "Draft saved")
	data, err := os.ReadFile(qfile)
	if err != nil {
		t.Fatal(err)
	}
	if code != http.StatusOK || !strings.Contains(string(data), "**Verdict:** b) on disk") || parseQuestions(string(data)).Answered {
		t.Fatalf("draft: %d\n%s", code, data)
	}
	after, err := os.Stat(qfile)
	if err != nil {
		t.Fatal(err)
	}
	if !os.SameFile(before, after) {
		t.Error("the questions file was replaced, not rewritten in place (the path unit watches it)")
	}
	// send with a question left empty: refused, nothing written
	code, body = post(t, ts, "/questions/T2", url.Values{"c0": {"ok"}, "c1": {"own"}, "t1": {"  "}, "action": {"send"}})
	data2, err := os.ReadFile(qfile)
	if err != nil {
		t.Fatal(err)
	}
	if code != http.StatusUnprocessableEntity || !strings.Contains(body, "without a verdict") || string(data2) != string(data) {
		t.Fatalf("incomplete: %d (file changed: %v)", code, string(data2) != string(data))
	}
	// send: marked ANSWERED, `agent _answers_saved T2 1` called (here it "resumes" the task)
	var called []string
	stubAgent(t, func(args ...string) (string, *OpError[AgentCallCause]) {
		called = args
		writeFile(t, filepath.Join(home, "tasks", "T2", "meta.env"), "ID=T2\nAGENT=asker\nSTATUS=starting\nRUN=2\nROUND=1\n")
		return "", nil
	})
	code, body = post(t, ts, "/questions/T2", url.Values{"c0": {"ok"}, "t0": {""}, "c1": {"own"}, "t1": {"parser"}, "action": {"send"}})
	data, err = os.ReadFile(qfile)
	if err != nil {
		t.Fatal(err)
	}
	if code != http.StatusOK || !strings.Contains(body, "Answers sent") || strings.Join(called, " ") != "_answers_saved T2 1" ||
		!strings.HasSuffix(string(data), "**ANSWERED**\n") || !strings.Contains(string(data), "**Verdict:** ok\n") {
		t.Fatalf("send: %d %v\n%s", code, called, data)
	}
}

func TestAnswerSendReportsAFailedResume(t *testing.T) {
	home := fixture(t)
	_, ts := testServer(t, home)
	stubAgent(t, func(_ ...string) (string, *OpError[AgentCallCause]) {
		return "agent busy", webErr(AgentCallExit, errors.New("exit status 3"), "agent _answers_saved: agent busy")
	})
	code, body := post(t, ts, "/questions/T2", url.Values{"c0": {"ok"}, "c1": {"own"}, "t1": {"x"}, "action": {"send"}})
	if code != AnswerResume.HTTPStatus() || !strings.Contains(body, "the answers are saved, but sending them failed") {
		t.Fatalf("%d\n%s", code, body)
	}
}

// readEvent reads server-sent events until one of the given name arrives; it returns its data.
func readEvent(t *testing.T, br *bufio.Reader, name string) string {
	t.Helper()
	cur := ""
	var data []string
	for {
		line, err := br.ReadString('\n')
		if err != nil {
			t.Fatalf("stream ended before %q: %v", name, err)
		}
		line = strings.TrimRight(line, "\n")
		switch {
		case strings.HasPrefix(line, "event: "):
			cur, data = strings.TrimPrefix(line, "event: "), nil
		case strings.HasPrefix(line, "data: "):
			data = append(data, strings.TrimPrefix(line, "data: "))
		case line == "" && cur == name:
			return strings.Join(data, "\n")
		}
	}
}

// openStream starts a server-sent event stream (cancelled at the end of the test).
func openStream(t *testing.T, ts *httptest.Server, path string) *bufio.Reader {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, ts.URL+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := ts.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := resp.Body.Close(); err != nil && !errors.Is(err, context.Canceled) {
			t.Logf("closing the stream: %v", err)
		}
	})
	if ct := resp.Header.Get("Content-Type"); ct != "text/event-stream" {
		t.Fatalf("content type %q", ct)
	}
	return bufio.NewReader(resp.Body)
}

func TestTaskStreamSendsNewActivity(t *testing.T) {
	home := fixture(t)
	_, ts := testServer(t, home)
	act := filepath.Join(home, "tasks", "T1", "activity.jsonl")
	st, err := os.Stat(act)
	if err != nil {
		t.Fatal(err)
	}
	br := openStream(t, ts, fmt.Sprintf("/events/task/T1?offset=%d", st.Size()))
	f, err := os.OpenFile(act, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(`{"ts":"2026-10-23 08:03:00","k":"edit","t":"edited <main.go>"}` + "\n"); err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	if got := readEvent(t, br, "act"); !strings.Contains(got, "edited &lt;main.go&gt;") || !strings.Contains(got, "k-edit") {
		t.Errorf("activity event (escaped): %q", got)
	}
}

func TestDashboardAndBusStreams(t *testing.T) {
	home := fixture(t)
	_, ts := testServer(t, home)
	if got := readEvent(t, openStream(t, ts, "/events/dashboard"), "table"); !strings.Contains(got, ">solo<") {
		t.Errorf("dashboard table event: %q", got)
	}
	msgs := filepath.Join(home, "bus", "messages.jsonl")
	st, err := os.Stat(msgs)
	if err != nil {
		t.Fatal(err)
	}
	br := openStream(t, ts, fmt.Sprintf("/events/bus?offset=%d&kind=chat", st.Size()))
	f, err := os.OpenFile(msgs, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	for _, l := range []string{
		`{"id":3,"ts":"2026-10-23 08:09:00","from":"solo","to":"all","kind":"checkpoint","text":"filtered out"}`,
		`{"id":4,"ts":"2026-10-23 08:10:00","from":"lead","to":"member","kind":"chat","text":"live message"}`,
	} {
		if _, err := f.WriteString(l + "\n"); err != nil {
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	if got := readEvent(t, br, "msg"); !strings.Contains(got, "live message") || strings.Contains(got, "filtered out") {
		t.Errorf("bus event: %q", got)
	}
}

func TestStaticFilesAndHeaders(t *testing.T) {
	s, ts := testServer(t, fixture(t))
	req, err := http.NewRequest(http.MethodGet, ts.URL+"/static/app.css?v="+s.assetV, nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := ts.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	closeBody(t, resp)
	if resp.StatusCode != http.StatusOK || !strings.Contains(resp.Header.Get("Cache-Control"), "immutable") {
		t.Errorf("static: %d %q", resp.StatusCode, resp.Header.Get("Cache-Control"))
	}
	_, body := get(t, ts, "/?max=1")
	mustContain(t, "maximized page", body, `<body class="max"`, "/static/vendor/htmx.min.js?v="+s.assetV)
}

func TestVendoredAssetsMatchSources(t *testing.T) {
	src, err := fs.ReadFile(webFS, "web/static/vendor/SOURCES")
	if err != nil {
		t.Fatal(err)
	}
	n := 0
	for _, l := range strings.Split(string(src), "\n") {
		f := strings.Fields(l)
		if len(f) < 2 || strings.HasPrefix(l, "#") {
			continue
		}
		data, err := fs.ReadFile(webFS, "web/static/vendor/"+f[0])
		if err != nil {
			t.Fatal(err)
		}
		sum := sha256.Sum256(data)
		if got := hex.EncodeToString(sum[:]); got != f[1] {
			t.Errorf("%s: sha256 %s, SOURCES says %s (update with tools/update-web-vendor.sh)", f[0], got, f[1])
		}
		n++
	}
	if n != 2 {
		t.Errorf("SOURCES lists %d files, want 2", n)
	}
}

func TestInsideDir(t *testing.T) {
	for path, want := range map[string]bool{
		"/h/tasks/T1/questions-1.md": true, "/h/tasks/../secrets.env": false, "/h/tasks": false, "/h/tasks2/x": false, "/etc/passwd": false,
	} {
		if got := insideDir("/h/tasks", path); got != want {
			t.Errorf("insideDir(/h/tasks, %s) = %v, want %v", path, got, want)
		}
	}
}

func TestAgentListFailuresAreShownNotHidden(t *testing.T) {
	_, ts := testServer(t, fixture(t))
	stubAgent(t, func(_ ...string) (string, *OpError[AgentCallCause]) {
		return "", webErr(AgentCallStart, errors.New("no such file"), "cannot run /nonexistent/agent")
	})
	code, body := get(t, ts, "/agents")
	if code != http.StatusOK {
		t.Fatalf("%d", code)
	}
	mustContain(t, "/agents", body, "The agent list could not be read", "cannot run /nonexistent/agent")
	if _, perr := parseAgents("not json"); perr == nil || !errors.Is(perr, ErrInvalidValue) {
		t.Errorf("parseAgents: %v", perr)
	}
}
