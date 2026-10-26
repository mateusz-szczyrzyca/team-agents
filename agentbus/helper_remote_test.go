package main

import (
	_ "embed"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// helperBeforeRemote is an actual pre-round-8 helper snapshot with no new fields.
//
//go:embed testdata/state-before-remote-helpers.json
var helperBeforeRemote []byte

// remoteHelperFixture creates a queued request without invoking the local dispatcher.
func remoteHelperFixture(t *testing.T, b *Bus) *Helper {
	t.Helper()
	h := &Helper{ID: 1, From: "member", FromTask: "task-member", Repo: "repo", Branch: "agents/member", Worktree: t.TempDir(), Request: "build this commit", Context: "return findings", Requires: []string{"hw:rp2350"}, BaseSHA: regSHA, Status: HelperQueued, Created: regAt(1)}
	mustWith(t, b, func(st *State) { st.NextHelperID = 1; st.Helpers["1"] = h })
	return h
}

// TestDelegateRequiresCapturesCommit: requirements are canonical and the SHA is captured before dispatch.
func TestDelegateRequiresCapturesCommit(t *testing.T) {
	b := newBus(t)
	s := server(b, "member")
	old := agentCmdFn
	agentCmdFn = func(_, _ string, _ ...string) {}
	t.Cleanup(func() { agentCmdFn = old })
	stubNodeGit(t, func(root string, args ...string) ([]byte, *ProcessError) {
		if root != s.info.Worktree || !reflect.DeepEqual(args, []string{"rev-parse", "--verify", "HEAD^{commit}"}) {
			t.Fatalf("unexpected snapshot: %s %q", root, args)
		}
		return []byte(regSHA + "\n"), nil
	})
	chk(t).ok(s.callTool("delegate", map[string]any{"request": "build", "requires": []any{"hw:rp2350", "os:linux", "hw:rp2350"}}))
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil || h.BaseSHA != regSHA || !reflect.DeepEqual(h.Requires, []string{"hw:rp2350", "os:linux"}) {
			t.Fatalf("snapshot not retained: %+v", h)
		}
	})
}

// TestDelegateRequiresCauses: malformed capabilities, snapshot failures and array types fail explicitly.
func TestDelegateRequiresCauses(t *testing.T) {
	s := server(newBus(t), "member")
	failsWith(t, DelegateInvalidCapabilities)(s.delegate("build", "", "sign:key"))
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return nil, processErr("git", nil, "snapshot failed") })
	failsWith(t, DelegateSnapshotFailed)(s.delegate("build", "", "os:linux"))
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return []byte("bad-sha"), nil })
	failsWith(t, DelegateSnapshotFailed)(s.delegate("build", "", "os:linux"))
	for _, value := range []any{"os:linux", []any{42}, nil} {
		_, err := s.callTool("delegate", map[string]any{"request": "build", "requires": value})
		expectToolError(t, err, ToolCallInvalidArgument)
	}
}

// TestHelperCapabilityRouting: only satisfied unbound requests can enter the local dispatcher.
func TestHelperCapabilityRouting(t *testing.T) {
	b := newBus(t)
	h := remoteHelperFixture(t, b)
	mustWith(t, b, func(st *State) {
		st.Helpers["2"] = &Helper{ID: 2, Status: HelperQueued}
		got := takeLocalHelper(st, "worker", []string{"os:linux"})
		if got == nil || got.ID != 2 || got.Status != HelperStarting || got.Worker != "worker" {
			t.Fatalf("local route=%+v", got)
		}
		st.Helpers["1"] = h
	})
	got, err := remoteHelpers(b, []string{"os:linux"})
	if err != nil || len(got) != 1 || got[0].ID != 1 {
		t.Fatalf("remote route=%+v, %v", got, err)
	}
	got, err = remoteHelpers(b, []string{"hw:rp2350"})
	if err != nil || len(got) != 0 {
		t.Fatalf("satisfied helper routed remotely: %+v, %v", got, err)
	}
	mustWith(t, b, func(st *State) {
		st.Helpers["1"].RemoteJob = &JobRequest{ID: "frozen"}
		if got := takeLocalHelper(st, "worker", []string{"hw:rp2350"}); got != nil {
			t.Fatalf("bound request stolen locally: %+v", got)
		}
	})
	got, err = remoteHelpers(b, []string{"hw:rp2350"})
	if err != nil || len(got) != 1 {
		t.Fatalf("bound request lost after capability change: %+v, %v", got, err)
	}
}

// TestRemoteHelperPrepareFreezesVerifiedRequest: a fresh remote branch must contain the captured SHA.
func TestRemoteHelperPrepareFreezesVerifiedRequest(t *testing.T) {
	b := newBus(t)
	h := remoteHelperFixture(t, b)
	var calls [][]string // fresh fetch, ancestry check and private-ref cleanup
	stubNodeGit(t, func(root string, args ...string) ([]byte, *ProcessError) {
		if root != h.Worktree {
			t.Fatalf("wrong worktree: %s", root)
		}
		calls = append(calls, args)
		return nil, nil
	})
	got, err := prepareRemoteHelper(b, 1, "node-a")
	if err != nil {
		t.Fatal(err)
	}
	if got.BaseSHA != regSHA || got.BaseRef != h.Branch || got.Requester != "member@node-a" || got.Selector != "worker" || got.Repository != "repo" || !strings.Contains(got.Task, h.Context) || got.ID == "" {
		t.Fatalf("wrong job: %+v", got)
	}
	if len(calls) != 3 || len(calls[0]) != 6 || calls[0][0] != "fetch" || calls[0][4] != "origin" {
		t.Fatalf("push not freshly verified: %q", calls)
	}
	_, ref, ok := strings.Cut(calls[0][5], ":refs/team-agents/")
	if !ok {
		t.Fatalf("fetch not isolated: %q", calls)
	}
	privateRef := "refs/team-agents/" + ref
	if calls[0][5] != "refs/heads/"+h.Branch+":"+privateRef || !reflect.DeepEqual(calls[1], []string{"merge-base", "--is-ancestor", regSHA, privateRef}) || !reflect.DeepEqual(calls[2], []string{"update-ref", "-d", privateRef}) {
		t.Fatalf("wrong push verification: %q", calls)
	}
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) {
		t.Fatal("retry must not reverify a frozen job")
		return nil, nil
	})
	gotAgain, retryErr := prepareRemoteHelper(b, 1, "node-a")
	if retryErr != nil || !reflect.DeepEqual(gotAgain, got) {
		t.Fatalf("retry changed job: %+v %v", gotAgain, retryErr)
	}
}

// TestRemoteHelperPrepareCauses: invalid metadata and unpushed commits never produce a job.
func TestRemoteHelperPrepareCauses(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	_, err := prepareRemoteHelper(b, 9, "node-a")
	failsWith(t, HelperRemotePrepareNoSuchRequest)("HelperRemotePrepareNoSuchRequest", err)
	_, err = prepareRemoteHelper(b, 1, "../bad")
	failsWith(t, HelperRemotePrepareInvalidRequest)("HelperRemotePrepareInvalidRequest", err)
	stubNodeGit(t, func(string, ...string) ([]byte, *ProcessError) { return nil, processErr("git", nil, "fetch failed") })
	_, err = prepareRemoteHelper(b, 1, "node-a")
	failsWith(t, HelperRemotePrepareNotPushed)("HelperRemotePrepareNotPushed", err)
	stubNodeGit(t, func(_ string, args ...string) ([]byte, *ProcessError) {
		if args[0] == "merge-base" {
			return nil, processErr("git", nil, "not ancestor")
		}
		return nil, nil
	})
	_, err = prepareRemoteHelper(b, 1, "node-a")
	failsWith(t, HelperRemotePrepareNotPushed)("HelperRemotePrepareNotPushed", err)
	mustWith(t, b, func(st *State) { st.Helpers["1"].Status = HelperStarting })
	_, err = prepareRemoteHelper(b, 1, "node-a")
	failsWith(t, HelperRemotePrepareInvalidRequest)("HelperRemotePrepareInvalidRequest", err)
}

// TestRemoteHelperResultStates: all terminal outcomes become ordinary helper-result messages exactly once on retries.
func TestRemoteHelperResultStates(t *testing.T) {
	for _, state := range []JobState{JobDone, JobFailed, JobStopped} {
		t.Run(string(state), func(t *testing.T) {
			b := newBus(t)
			remoteHelperFixture(t, b)
			result := remoteHelperResult{State: state, Node: "node-b", Branch: "agents/result", PRURL: "https://example.com/pr/1", Result: "verified"}
			if err := finishRemoteHelper(b, 1, result); err != nil {
				t.Fatal(err)
			}
			if err := finishRemoteHelper(b, 1, result); err != nil {
				t.Fatal(err)
			}
			msgs, err := b.messages()
			if err != nil || len(msgs) != 1 {
				t.Fatalf("messages=%+v err=%v", msgs, err)
			}
			var event EventScript // ordinary script notice consumed by existing clients
			if err := json.Unmarshal(msgs[0].Data, &event); err != nil {
				t.Fatal(err)
			}
			if msgs[0].To != "member" || event.Notice != NoticeHelperResult || event.Fields["status"] != string(state) || !strings.Contains(msgs[0].Text, "node-b") || !strings.Contains(msgs[0].Text, "agents/result") || !strings.Contains(msgs[0].Text, "verified") {
				t.Fatalf("wrong result: %+v", msgs[0])
			}
			mustWith(t, b, func(st *State) {
				want := HelperFailed
				if state == JobDone {
					want = HelperDone
				}
				if st.Helpers["1"].Status != want || st.Helpers["1"].Summary == "" {
					t.Fatalf("unfinished helper: %+v", st.Helpers["1"])
				}
			})
		})
	}
}

// TestRemoteHelperFinishCauses: malformed terminal updates and missing requests have precise causes.
func TestRemoteHelperFinishCauses(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	failsWith(t, HelperRemoteFinishInvalidState)("nonterminal", finishRemoteHelper(b, 1, remoteHelperResult{State: JobRunning}))
	failsWith(t, HelperRemoteFinishNoSuchRequest)("missing helper", finishRemoteHelper(b, 9, remoteHelperResult{State: JobDone}))
}

// TestRemoteHelperStorageCauses: every local adapter preserves storage failures as its own cause.
func TestRemoteHelperStorageCauses(t *testing.T) {
	b := &Bus{dir: filepath.Join(t.TempDir(), "blocked")}
	if err := os.WriteFile(b.dir, []byte("file"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, lerr := remoteHelpers(b, nil)
	failsWith(t, HelperRemoteListStorage)("list", lerr)
	_, perr := prepareRemoteHelper(b, 1, "node-a")
	failsWith(t, HelperRemotePrepareStorage)("prepare", perr)
	failsWith(t, HelperRemoteFinishStorage)("finish", finishRemoteHelper(b, 1, remoteHelperResult{State: JobDone}))
}

// TestRemoteHelperListRejectsCapabilities: invalid local declarations cannot redirect requests.
func TestRemoteHelperListRejectsCapabilities(t *testing.T) {
	_, err := remoteHelpers(newBus(t), []string{"sign:key"})
	failsWith(t, HelperRemoteListInvalidCapabilities)("invalid local capabilities", err)
}

// TestRemoteHelperResultSurvivesStateSaveFailure: an already posted result is neither duplicated nor replaced.
func TestRemoteHelperResultSurvivesStateSaveFailure(t *testing.T) {
	b := newBus(t)
	remoteHelperFixture(t, b)
	blocked := b.path("state.json.tmp")
	if err := os.Mkdir(blocked, 0o700); err != nil {
		t.Fatal(err)
	}
	failsWith(t, HelperRemoteFinishStorage)("state save", finishRemoteHelper(b, 1, remoteHelperResult{State: JobDone, Node: "node-b", Result: "first result"}))
	if err := os.Remove(blocked); err != nil {
		t.Fatal(err)
	}
	if err := finishRemoteHelper(b, 1, remoteHelperResult{State: JobFailed, Result: "changed retry"}); err != nil {
		t.Fatal(err)
	}
	mustWith(t, b, func(st *State) {
		if st.Helpers["1"].Status != HelperDone || !strings.Contains(st.Helpers["1"].Summary, "first result") {
			t.Fatalf("posted outcome replaced: %+v", st.Helpers["1"])
		}
	})
	chk(t).ok(toolResult(server(b, "member").send("other", "next message")))
	msgs, err := b.messages()
	if err != nil || len(msgs) != 2 || msgs[0].ID >= msgs[1].ID {
		t.Fatalf("result replay or reused message ID: %+v %v", msgs, err)
	}
}

// TestRemoteHelperLegacyFixture: helpers written before requirements existed still load as ordinary local requests.
func TestRemoteHelperLegacyFixture(t *testing.T) {
	b := installState(t, helperBeforeRemote)
	mustWith(t, b, func(st *State) {
		h := st.Helpers["1"]
		if h == nil || len(h.Requires) != 0 || h.BaseSHA != "" || h.RemoteJob != nil {
			t.Fatalf("legacy helper changed: %+v", h)
		}
	})
}
