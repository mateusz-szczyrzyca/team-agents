package main

import (
	"path/filepath"
	"testing"
)

// setupMirroredTaskFixture creates a test home with a mirrored task under bus/remote/node-a/T-rem-1.
func setupMirroredTaskFixture(t *testing.T) string {
	t.Helper()
	home := fixture(t)
	dir := filepath.Join(home, "bus", "remote", "node-a", "T-rem-1")
	writeFile(t, filepath.Join(dir, "metadata.json"), `{
  "STATUS": "running",
  "RUN": "1",
  "AGENT": "worker-1",
  "TYPE": "worker",
  "MODEL": "model-x",
  "NOTE": "testing mirrored task",
  "CREATED": "2026-10-28T12:00:00Z",
  "RUN_START": "2026-10-28T12:00:01Z",
  "JOB_ID": "job-rem-1",
  "RESTARTS": "0"
}`)
	writeFile(t, filepath.Join(dir, "activity.jsonl"), `{"ts":"2026-10-28 12:00:05","k":"cmd","t":"make gate"}`+"\n"+`{"ts":"2026-10-28 12:00:10","k":"text","t":"compilation successful"}`+"\n")
	writeFile(t, filepath.Join(dir, "prompt-1.md"), "# Mirrored task prompt")
	writeFile(t, filepath.Join(dir, "questions-1.md"), "# Mirrored questions")
	writeFile(t, filepath.Join(dir, "final.md"), "# Mirrored final report")
	writeFile(t, filepath.Join(dir, "err-1.log"), "mirror error log output")
	return home
}

// TestMirroredTaskPageTabs verifies that a mirrored task page (/task/<id>@<node>)
// displays the mirrored tabs and shows unmirrored notices for investigations and notebook.
func TestMirroredTaskPageTabs(t *testing.T) {
	home := setupMirroredTaskFixture(t)
	_, ts := testServer(t, home)

	// Activity tab (default)
	status, body := get(t, ts, "/task/T-rem-1@node-a")
	if status != 200 {
		t.Fatalf("GET /task/T-rem-1@node-a: want status 200, got %d (body: %s)", status, body)
	}
	mustContain(t, "/task/T-rem-1@node-a", body, "worker-1@node-a", "make gate", "compilation successful")

	// Prompt tab
	_, bodyPrompt := get(t, ts, "/task/T-rem-1@node-a?tab=prompt")
	mustContain(t, "/task/T-rem-1@node-a?tab=prompt", bodyPrompt, "Mirrored task prompt")

	// Questions tab
	_, bodyQ := get(t, ts, "/task/T-rem-1@node-a?tab=questions")
	mustContain(t, "/task/T-rem-1@node-a?tab=questions", bodyQ, "Mirrored questions")

	// Report tab
	_, bodyReport := get(t, ts, "/task/T-rem-1@node-a?tab=report")
	mustContain(t, "/task/T-rem-1@node-a?tab=report", bodyReport, "Mirrored final report")

	// Errors tab
	_, bodyErr := get(t, ts, "/task/T-rem-1@node-a?tab=errors")
	mustContain(t, "/task/T-rem-1@node-a?tab=errors", bodyErr, "mirror error log output")

	// Meta tab
	_, bodyMeta := get(t, ts, "/task/T-rem-1@node-a?tab=meta")
	mustContain(t, "/task/T-rem-1@node-a?tab=meta", bodyMeta, "JOB_ID", "job-rem-1", "worker-1")

	// Investigations tab: unmirrored notice
	_, bodyInv := get(t, ts, "/task/T-rem-1@node-a?tab=investigations")
	mustContain(t, "/task/T-rem-1@node-a?tab=investigations", bodyInv, "not mirrored to the lead")

	// Notebook tab: unmirrored notice
	_, bodyNotes := get(t, ts, "/task/T-rem-1@node-a?tab=notebook")
	mustContain(t, "/task/T-rem-1@node-a?tab=notebook", bodyNotes, "not mirrored to the lead")
}
