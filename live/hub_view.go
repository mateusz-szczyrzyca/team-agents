// Hub-backed panels consume the authenticated agentbus snapshot command, sharing its TLS policy.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// hubViewData mirrors the public display schema without importing agentbus or reading its state files.
type hubViewData struct {
	Version        int                       `json:"version"`        // currently version 1
	At             string                    `json:"at"`             // snapshot UTC instant
	Instances      []hubViewInstance         `json:"instances"`      // manifests without lease credentials
	Agents         map[string]*Presence      `json:"agents"`         // MCP stages
	Teams          map[string]*TeamInfo      `json:"teams"`          // boards and member states
	Investigations map[string]*Investigation `json:"investigations"` // queue summaries; remote record files omitted
	Messages       []Message                 `json:"messages"`       // recent bus messages
}

// hubViewInstance contains only display metadata, never executable paths or session credentials.
type hubViewInstance struct {
	Name         string                 `json:"name"`         // instance identity
	Active       bool                   `json:"active"`       // current lease
	ExpiresAt    string                 `json:"expires_at"`   // lease deadline
	OS           string                 `json:"os"`           // detected OS
	Arch         string                 `json:"arch"`         // detected architecture
	Capabilities []string               `json:"capabilities"` // effective closed vocabulary
	Slots        instanceSlots          `json:"slots"`        // current capacity
	Tasks        map[string]hubViewTask `json:"tasks"`        // node-owned task summaries
}

// hubViewTask intentionally ignores remote Worktree: no returned pathname grants local filesystem access.
type hubViewTask struct {
	ID        string    `json:"task_id"`    // raw task ID on its owning node
	Agent     string    `json:"agent"`      // base agent name
	Team      string    `json:"team"`       // team ID
	Repo      string    `json:"repo"`       // repository name
	Branch    string    `json:"branch"`     // pushed branch
	Tier      string    `json:"tier"`       // normal or worker
	Role      string    `json:"role"`       // registered role
	Status    string    `json:"status"`     // runner state
	StartedAt time.Time `json:"started_at"` // task start
	UpdatedAt time.Time `json:"updated_at"` // last report activity
}

// hubViewSource owns the operator-selected helper path and the last successful overview.
type hubViewSource struct {
	program string    // configured agentbus binary, never a hub-supplied executable
	home    string    // local agents home for credentials/logging
	busDir  string    // local socket root when viewing the lead
	last    *Snapshot // last successful snapshot, retained during outages
}

// poll preserves the last hub overview on outages, with an explicit stale-view flag and no local-file fallback.
func (source *hubViewSource) poll() *Snapshot {
	snapshot, err := source.read()
	if err == nil {
		source.last = snapshot
		return snapshot
	}
	warnOnce("hub-view", LogHubViewUnavailable, err)
	snapshot = &Snapshot{At: time.Now(), All: map[string]*Task{}, Agents: map[string]*Presence{}, Teams: map[string]*TeamInfo{}}
	if source.last != nil {
		retained := *source.last
		snapshot = &retained
	}
	snapshot.HubError = err.Message
	return snapshot
}

// read executes only the configured read-only helper, with bounded time and output.
func (source *hubViewSource) read() (*Snapshot, *StorageError) {
	if !filepath.IsAbs(source.program) {
		return nil, storageErr(fmt.Errorf("snapshot helper must be absolute"), "cannot load hub view")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, source.program, "snapshot", "--dir", source.busDir, "--timeout", "2") //nolint:gosec // G204: operator-selected agentbus path and fixed read-only arguments, never hub-provided executable text
	cmd.WaitDelay = time.Second                                                                           // bound output-copy cleanup if a helper leaves an inherited pipe open
	cmd.Env = append(os.Environ(), "AGENTS_HOME="+source.home)
	var output hubViewOutput
	cmd.Stdout = &output
	if err := cmd.Run(); err != nil {
		return nil, storageErr(err, "cannot read hub overview")
	}
	return decodeHubView(output.data)
}

// decodeHubView creates metadata-only task entries; paths in node reports are never opened locally.
func decodeHubView(data []byte) (*Snapshot, *StorageError) {
	var view hubViewData
	if err := json.Unmarshal(data, &view); err != nil {
		return nil, storageErr(err, "cannot decode hub view")
	}
	if view.Version != 1 {
		return nil, storageErr(fmt.Errorf("unsupported snapshot version %d", view.Version), "cannot decode hub view")
	}
	at, err := time.Parse(time.RFC3339Nano, view.At)
	if err != nil {
		return nil, storageErr(err, "cannot read hub snapshot time")
	}
	snapshot := &Snapshot{At: at, All: map[string]*Task{}, Agents: view.Agents, Teams: view.Teams, Msgs: view.Messages}
	for _, node := range view.Instances {
		expires, err := parseInstanceTime(node.ExpiresAt)
		if err != nil {
			return nil, storageErr(err, "cannot read instance lease time")
		}
		snapshot.Instances = append(snapshot.Instances, InstanceInfo{Name: node.Name, Active: node.Active, ExpiresAt: expires,
			OS: node.OS, Arch: node.Arch, SlotsFree: node.Slots.Free, SlotsTotal: node.Slots.Total, Tasks: len(node.Tasks), Capabilities: node.Capabilities})
		for id, summary := range node.Tasks {
			if summary.ID != id || id == "" || strings.ContainsAny(id, "/\\@") || node.Name == "" || strings.ContainsAny(node.Name, "/\\@") {
				return nil, storageErr(fmt.Errorf("invalid task or instance identifier"), "cannot decode hub view")
			}
			status := summary.Status
			if !node.Active && activeStatuses[status] {
				status = statusUnreachable
			}
			task := &Task{ID: id + "@" + node.Name, Agent: summary.Agent + "@" + node.Name, Instance: node.Name,
				Remote: true, Kind: "worker", Team: summary.Team, Repo: summary.Repo, Branch: summary.Branch,
				Tier: summary.Tier, Status: status, Created: summary.StartedAt, Alive: node.Active && status == "running"}
			if summary.Role == "investigator" {
				task.Kind = "investigation"
			}
			snapshot.All[task.ID] = task
			snapshot.Tasks = append(snapshot.Tasks, task)
		}
	}
	for _, investigation := range view.Investigations {
		snapshot.Invs = append(snapshot.Invs, investigation)
	}
	sort.Slice(snapshot.Invs, func(i, j int) bool { return snapshot.Invs[i].ID < snapshot.Invs[j].ID })
	sort.Slice(snapshot.Instances, func(i, j int) bool { return snapshot.Instances[i].Name < snapshot.Instances[j].Name })
	sort.Slice(snapshot.Tasks, func(i, j int) bool {
		left, right := statusPrio(snapshot.Tasks[i]), statusPrio(snapshot.Tasks[j])
		if left != right {
			return left < right
		}
		return snapshot.Tasks[i].ID < snapshot.Tasks[j].ID
	})
	return snapshot, nil
}

// hubViewOutput bounds a helper's stdout before allocating more memory.
type hubViewOutput struct {
	data []byte // collected JSON, capped at 64 MiB
}

// Write implements io.Writer; overflow returns the standard io.ErrShortBuffer sentinel.
func (output *hubViewOutput) Write(data []byte) (int, error) {
	if len(data) > (64<<20)-len(output.data) {
		return 0, io.ErrShortBuffer
	}
	output.data = append(output.data, data...)
	return len(data), nil
}
