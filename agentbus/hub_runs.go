// Trusted run resolution binds authenticated instance reports to hub-owned assignments.
package main

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
)

// hubRunRegistry verifies assignments without opening a worktree from another instance.
type hubRunRegistry struct {
	bus           *Bus                 // hub-owned state, never selected by the caller
	instances     *HubInstanceRegistry // authenticated reports; nil denies remote runs
	localInstance string               // only this instance may use the local task directory
	tasksDir      string               // local metadata root; empty denies unregistered local solo runs
}

// resolve rechecks live reports and assignments on every request, including replay attachment.
func (r hubRunRegistry) resolve(c HubCaller) (*Server, *HubCallError) {
	if r.bus == nil || c.Agent == "" || strings.Contains(c.Agent, "@") || !validInstanceName(c.Instance) || !validNodeTaskID(c.Task) || !c.Role.Valid() || !c.Tier.Valid() {
		return nil, opErr(HubCallIdentity, "caller must identify a registered run")
	}
	if c.Task == "cli" || c.Agent == "cli" {
		return &Server{bus: r.bus, me: c.Agent + "@" + c.Instance, instance: c.Instance, role: c.Role, tier: c.Tier}, nil
	}
	var summary HubTaskSummary
	if c.Instance == r.localInstance {
		var err *HubCallError
		summary, err = r.localTask(c.Task)
		if err != nil {
			return nil, err
		}
	}
	var result *HubCallError
	serr := r.bus.locked(func(st *State) {
		if c.Instance != r.localInstance {
			if r.instances == nil || r.instances.Bus != r.bus {
				result = opErr(HubCallIdentity, "instance registry is not configured")
				return
			}
			records, err := r.instances.load()
			if err != nil {
				result = fromStorage(HubCallStorage, err)
				return
			}
			record := records.Instances[c.Instance]
			if record == nil || !r.instances.now().Before(record.Lease.ExpiresAt.Time) {
				result = opErr(HubCallIdentity, "instance %s has no live registration", c.Instance)
				return
			}
			summary = record.Manifest.CurrentState.Tasks[c.Task]
		}
		if summary.TaskID != c.Task || summary.Agent != c.Agent || summary.Team != c.Team || summary.Tier != c.Tier || summary.Role != c.Role || summary.Repo == "" || (!nodeTaskReservesAgent(summary.Status) && summary.Status != nodeTaskWaiting) {
			result = opErr(HubCallIdentity, "run %s does not match its registered identity", c.Task)
			return
		}
		if !r.assigned(st, c, summary) {
			result = opErr(HubCallIdentity, "run %s has no matching active assignment", c.Task)
		}
	})
	if serr != nil {
		return nil, fromStorage(HubCallStorage, serr)
	}
	if result != nil {
		return nil, result
	}
	qualified := c.Agent + "@" + c.Instance
	s := &Server{bus: r.bus, me: qualified, info: Presence{Agent: qualified, Task: c.Task, Repo: summary.Repo, Branch: summary.Branch, Worktree: summary.Worktree}, team: c.Team, tier: c.Tier, role: c.Role, instance: c.Instance, queueMax: 3}
	if r.instances != nil {
		s.directory = registryDirectory{registry: r.instances}
	}
	// Remote metadata is never used as a local records directory or executable path.
	if c.Instance == r.localInstance {
		s.tasksDir = r.tasksDir
	} else {
		s.tasksDir = filepath.Join(r.bus.dir, "remote-tasks", c.Instance)
	}
	return s, nil
}

// assigned checks the separate team/helper/investigation authorization, beyond instance ownership.
func (r hubRunRegistry) assigned(st *State, c HubCaller, summary HubTaskSummary) bool {
	qualified := c.Agent + "@" + c.Instance
	if c.Team != "" {
		team := st.Teams[c.Team]
		if team == nil || team.Done || c.Role != RoleAgent {
			return false
		}
		member := team.Members[qualified]
		return member != nil && member.Agent == qualified && member.Task == c.Task && member.Tier == c.Tier && member.State != MemberExcluded && team.Repo == summary.Repo && member.Branch == summary.Branch && member.Worktree == summary.Worktree
	}
	if c.Role == RoleInvestigator {
		if c.Tier != TierNormal {
			return false
		}
		for _, inv := range st.Investigations {
			if inv != nil && inv.InvTask == c.Task && inv.Repo == summary.Repo && ((inv.Investigator == c.Agent && inv.Instance == c.Instance) || (c.Instance == r.localInstance && inv.Investigator == "" && inv.Instance == "")) && (inv.Status == InvStarting || inv.Status == InvRunning || inv.Status == InvWaitingHuman) {
				return true
			}
		}
		return false
	}
	if c.Tier == TierWorker {
		for _, helper := range st.Helpers {
			if helper != nil && helper.HelperTask == c.Task && helper.Worker == qualified && helper.Repo == summary.Repo && (helper.Status == HelperStarting || helper.Status == HelperRunning) {
				return true
			}
		}
		return false
	}
	return true // Normal solo runs are authorized by their instance's registered task metadata.
}

// localTask reads only a validated lead-local task; missing or escaped paths deny identity.
func (r hubRunRegistry) localTask(task string) (HubTaskSummary, *HubCallError) {
	var summary HubTaskSummary
	if r.tasksDir == "" {
		return summary, opErr(HubCallIdentity, "local task directory is not configured")
	}
	root, err := filepath.EvalSymlinks(r.tasksDir)
	if errors.Is(err, os.ErrNotExist) {
		return summary, opErr(HubCallIdentity, "local run is not registered")
	}
	if err != nil {
		return summary, fromStorage(HubCallStorage, storageErr(err, "cannot inspect local task root"))
	}
	path, err := filepath.EvalSymlinks(filepath.Join(root, task, "meta.env"))
	if errors.Is(err, os.ErrNotExist) {
		return summary, opErr(HubCallIdentity, "local run is not registered")
	}
	if err != nil {
		return summary, fromStorage(HubCallStorage, storageErr(err, "cannot inspect local task metadata"))
	}
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
		return summary, opErr(HubCallIdentity, "local task metadata escapes its root")
	}
	file, err := os.Open(path) //nolint:gosec // G304: validated task identifier and resolved metadata path are constrained to the configured local task root
	if err != nil {
		return summary, fromStorage(HubCallStorage, storageErr(err, "cannot open local task metadata"))
	}
	defer closeHubFile(file)
	info, err := file.Stat()
	if err != nil {
		return summary, fromStorage(HubCallStorage, storageErr(err, "cannot inspect local metadata file"))
	}
	if !info.Mode().IsRegular() {
		return summary, opErr(HubCallIdentity, "local task metadata must be a regular file")
	}
	data, err := io.ReadAll(io.LimitReader(file, hubBodyLimit+1))
	if err != nil {
		return summary, fromStorage(HubCallStorage, storageErr(err, "cannot read local task metadata"))
	}
	if len(data) > hubBodyLimit {
		return summary, opErr(HubCallIdentity, "local task metadata is too large")
	}
	fields := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		key, value, found := strings.Cut(line, "=")
		switch key {
		case "ID", "AGENT", "TEAM", "TIER", "KIND", "REPO_NAME", "BRANCH", "WORKTREE", "STATUS":
			decoded, valid := decodeTaskValue(value)
			if !found || !valid {
				return summary, opErr(HubCallIdentity, "invalid local task metadata field %s", key)
			}
			if _, duplicate := fields[key]; duplicate {
				return summary, opErr(HubCallIdentity, "duplicate local task metadata field %s", key)
			}
			fields[key] = decoded
		}
	}
	tier := Tier(fields["TIER"])
	if tier == "" {
		tier = TierNormal
	} // Existing solo metadata predates the explicit normal tier.
	role := RoleAgent
	if fields["KIND"] == "investigation" {
		role = RoleInvestigator
	}
	return HubTaskSummary{Status: fields["STATUS"], TaskID: fields["ID"], Agent: fields["AGENT"], Team: fields["TEAM"], Repo: fields["REPO_NAME"], Branch: fields["BRANCH"], Worktree: fields["WORKTREE"], Tier: tier, Role: role}, nil
}

// decodeTaskValue accepts Bash printf-%q values without evaluating shell expansions or commands.
func decodeTaskValue(raw string) (string, bool) {
	if raw == "''" {
		return "", true
	}
	if strings.HasPrefix(raw, "$'") && strings.HasSuffix(raw, "'") {
		raw = raw[2 : len(raw)-1]
		var out strings.Builder
		for raw != "" {
			if strings.HasPrefix(raw, `\E`) || strings.HasPrefix(raw, `\e`) {
				out.WriteByte(27)
				raw = raw[2:]
				continue
			}
			value, multibyte, tail, err := strconv.UnquoteChar(raw, '\'')
			if err != nil {
				return "", false
			}
			if multibyte {
				out.WriteRune(value)
			} else {
				if value < 0 || value > 255 {
					return "", false
				}
				out.WriteByte(byte(value))
			}
			raw = tail
		}
		return out.String(), true
	}
	var out strings.Builder
	escaped := false
	for _, value := range raw {
		if escaped {
			out.WriteRune(value)
			escaped = false
			continue
		}
		if value == '\\' {
			escaped = true
			continue
		}
		if unicode.IsSpace(value) || strings.ContainsRune("$`'\";|&<>(){}", value) {
			return "", false
		}
		out.WriteRune(value)
	}
	return out.String(), !escaped
}

// authorizePeer prevents a different allowed node from borrowing an already-registered alias.
func (r hubRunRegistry) authorizePeer(instance, peer string) *HubCallError {
	if peer != "" && instance == r.localInstance && peer == "local:"+r.localInstance {
		return nil
	}
	if r.instances == nil || r.instances.Bus != r.bus {
		return opErr(HubCallIdentity, "instance registry is not configured")
	}
	var result *HubCallError
	serr := r.instances.withState(func(st *hubInstanceState) *StorageError {
		record := st.Instances[instance]
		if peer == "" || record == nil || record.Peer != peer || !r.instances.now().Before(record.Lease.ExpiresAt.Time) {
			result = opErr(HubCallIdentity, "authenticated peer does not own a live instance registration")
		}
		return nil
	})
	if serr != nil {
		return fromStorage(HubCallStorage, serr)
	}
	return result
}
