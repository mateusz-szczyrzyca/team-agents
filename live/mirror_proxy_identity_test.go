package main

import (
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestMirrorAndLocalRemoteMemberProxySingleRowAndDeterministicJobLink verifies that a mirrored
// remote-member task and its local remote-member proxy produce a single dashboard/panel row, keep
// the mirrored task URL reachable, and deterministically resolve the local proxy on /jobs.
func TestMirrorAndLocalRemoteMemberProxySingleRowAndDeterministicJobLink(t *testing.T) {
	home := fixture(t)
	now := time.Now().UTC()

	writeFile(t, filepath.Join(home, "tasks", "T_PROXY", "meta.env"),
		"ID=T_PROXY\nAGENT=worker-1@node-a\nSTATUS=running\nRUN=1\nKIND=remote-member\nTEAM=TT-team\nJOB_ID=job-proxy-member\n")

	mirrorDir := filepath.Join(home, "bus", "remote", "node-a", "T1")
	writeFile(t, filepath.Join(mirrorDir, "metadata.json"),
		`{"AGENT":"worker-1","TYPE":"codex","MODEL":"example-model","STATUS":"running","RUN":"1","CREATED":"2026-10-29T08:00:00Z","RUN_START":"2026-10-29T08:01:00Z","JOB_ID":"job-proxy-member"}`)
	writeFile(t, filepath.Join(mirrorDir, "activity.jsonl"),
		`{"ts":"08:02:00","k":"cmd","t":"running remote member task"}`+"\n")

	jobsJSON := fmt.Sprintf(`{"Version":1,"Jobs":[
		{"Request":{"ID":"job-proxy-member","Repository":"team-agents","Selector":"worker-1","Team":"TT-team"},"State":"running","Node":"node-a","TaskID":"T1","Branch":"agents/worker-1/t1"}
	],"Nodes":{"node-a":{"Heartbeat":{"Node":"node-a","WebURL":"https://example.com/node-a/"},"LastSeen":%q}}}`,
		now.Format(time.RFC3339Nano))
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), jobsJSON)

	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	if snap.All["T_PROXY"] == nil {
		t.Fatal("local proxy task T_PROXY missing from snap.All")
	}
	if snap.All["T1@node-a"] == nil {
		t.Fatal("mirrored task T1@node-a must remain addressable in snap.All")
	}

	var panelMatches []string
	for _, task := range snap.Tasks {
		if task.JobID == "job-proxy-member" {
			panelMatches = append(panelMatches, task.ID)
		}
	}
	if len(panelMatches) != 1 || panelMatches[0] != "T_PROXY" {
		t.Errorf("panel snap.Tasks for job-proxy-member = %v, want [T_PROXY]", panelMatches)
	}

	srv, ts := testServer(t, home)
	dash := srv.dashboard(srv.snapshot())
	var dashMatches []string
	for _, row := range dash.Rows {
		if row.ID == "T_PROXY" || row.ID == "T1@node-a" || row.ID == "job-proxy-member" {
			dashMatches = append(dashMatches, row.ID)
		}
	}
	if len(dashMatches) != 1 || dashMatches[0] != "T_PROXY" {
		t.Errorf("dashboard rows for proxied member = %v, want [T_PROXY]", dashMatches)
	}

	codeMirror, bodyMirror := get(t, ts, "/task/T1@node-a")
	if codeMirror != http.StatusOK {
		t.Fatalf("GET /task/T1@node-a: status %d, body %.200s", codeMirror, bodyMirror)
	}
	mustContain(t, "/task/T1@node-a", bodyMirror, "worker-1@node-a", "running remote member task")

	// Add local wrong-kind, wrong-team, empty-team, and larger-ID competitors to verify strict and deterministic selection.
	writeFile(t, filepath.Join(home, "tasks", "A_WRONG_KIND", "meta.env"),
		"ID=A_WRONG_KIND\nAGENT=worker-1@node-a\nSTATUS=running\nRUN=1\nKIND=worker\nTEAM=TT-team\nJOB_ID=job-proxy-member\n")
	writeFile(t, filepath.Join(home, "tasks", "A_WRONG_TEAM", "meta.env"),
		"ID=A_WRONG_TEAM\nAGENT=worker-1@node-a\nSTATUS=running\nRUN=1\nKIND=remote-member\nTEAM=OTHER-team\nJOB_ID=job-proxy-member\n")
	writeFile(t, filepath.Join(home, "tasks", "A_EMPTY_TEAM", "meta.env"),
		"ID=A_EMPTY_TEAM\nAGENT=worker-1@node-a\nSTATUS=running\nRUN=1\nKIND=remote-member\nJOB_ID=job-proxy-member\n")
	writeFile(t, filepath.Join(home, "tasks", "Z_PROXY_SECOND", "meta.env"),
		"ID=Z_PROXY_SECOND\nAGENT=worker-1@node-a\nSTATUS=running\nRUN=1\nKIND=remote-member\nTEAM=TT-team\nJOB_ID=job-proxy-member\n")

	jobsSnap, err := readWebJobs(filepath.Join(home, "bus"))
	if err != nil || len(jobsSnap.Jobs) != 1 {
		t.Fatalf("readWebJobs: err=%v len=%d", err, len(jobsSnap.Jobs))
	}
	for i := 0; i < 200; i++ {
		localSnap := loadSnapshot(home, filepath.Join(home, "bus"))
		row := buildWebJobRow(jobsSnap.Jobs[0], jobsSnap, localSnap, now, 90*time.Second)
		if row.ProxyTask != "T_PROXY" || row.ProxyTaskLink != "/task/T_PROXY" {
			t.Fatalf("iteration %d: buildWebJobRow picked ProxyTask=%q ProxyTaskLink=%q, want T_PROXY (/task/T_PROXY)",
				i, row.ProxyTask, row.ProxyTaskLink)
		}
	}
}

// TestMirroredSameNamedAgentNotMarkedCoordinator verifies that when a local agent leads a team,
// a mirrored remote member with the same unqualified AGENT name is not marked as coordinator
// in either web task rows or the terminal panel table.
func TestMirroredSameNamedAgentNotMarkedCoordinator(t *testing.T) {
	home := fixture(t)
	now := time.Now().UTC()

	writeFile(t, filepath.Join(home, "tasks", "T_LOCAL_LEAD", "meta.env"),
		"ID=T_LOCAL_LEAD\nAGENT=w1\nSTATUS=running\nRUN=1\nTEAM=TT-team\nREPO_NAME=demo\n")

	mirrorDir := filepath.Join(home, "bus", "remote", "n1", "T_REM_MEMBER")
	writeFile(t, filepath.Join(mirrorDir, "metadata.json"),
		`{"AGENT":"w1","TYPE":"codex","STATUS":"running","RUN":"1","CREATED":"2026-10-29T08:00:00Z","RUN_START":"2026-10-29T08:01:00Z","JOB_ID":"job-rem-w1"}`)

	jobsJSON := fmt.Sprintf(`{"Version":1,"Jobs":[
		{"Request":{"ID":"job-rem-w1","Repository":"demo","Selector":"w1","Team":"TT-team"},"State":"running","Node":"n1","TaskID":"T_REM_MEMBER"}
	],"Nodes":{"n1":{"Heartbeat":{"Node":"n1"},"LastSeen":%q}}}`,
		now.Format(time.RFC3339Nano))
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), jobsJSON)

	stateJSON := `{"teams":{"TT-team":{"id":"TT-team","lead":"w1","done":false,"members":{"w1":{"state":"active","tier":"normal","task":"T_LOCAL_LEAD"},"w1@n1":{"state":"active","tier":"normal","task":"T_REM_MEMBER@n1"}}}}}`
	writeFile(t, filepath.Join(home, "bus", "state.json"), stateJSON)

	snap := loadSnapshot(home, filepath.Join(home, "bus"))
	localTask := snap.All["T_LOCAL_LEAD"]
	remoteTask := snap.All["T_REM_MEMBER@n1"]
	if localTask == nil || remoteTask == nil {
		t.Fatalf("tasks missing: local=%v remote=%v", localTask, remoteTask)
	}

	localWebRow := makeRow(snap, localTask)
	remoteWebRow := makeRow(snap, remoteTask)
	if !localWebRow.Lead {
		t.Errorf("local coordinator makeRow.Lead = false, want true")
	}
	if remoteWebRow.Lead {
		t.Errorf("mirrored same-named member makeRow.Lead = true, want false")
	}

	m := model{snap: snap, width: 120, height: 40}
	localPanelLine := m.row(localTask, 120)
	remotePanelLine := m.row(remoteTask, 120)
	if !strings.Contains(localPanelLine, "★") {
		t.Errorf("local coordinator panel row %q missing ★ marker", localPanelLine)
	}
	if strings.Contains(remotePanelLine, "★") {
		t.Errorf("mirrored same-named member panel row %q falsely contains ★ coordinator marker", remotePanelLine)
	}
}
