// Remote team members: a member that works on a node is a member of a team on the lead, the team's only bus. Its
// node relays its MCP messages to the lead (POST /v1/mcp); the lead runs them with the same code as a local
// member's (Server.handle), with the identity its grant gives — never one a request claims.
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strings"
	"time"
)

// remoteUnavailable are the tools a remote member does not get, and why (they are refused and not listed).
var remoteUnavailable = map[string]string{
	"reserve":               "resources are reserved on the lead's machine, not on yours",
	"release":               "resources are reserved on the lead's machine, not on yours",
	"claim_lead":            "the coordinator is always local to the lead",
	"handover_lead":         "the coordinator is always local to the lead",
	"delegate":              "helpers work on the lead's copy of a branch — ask the coordinator to delegate",
	"request_investigation": "investigators work on the lead's copy of a branch — ask the coordinator to request one",
	"wait_for_message":      "end your turn with STATUS: SLEEPING instead — you are woken when a message comes",
}

// withoutRemoteUnavailable drops the tools a remote member does not get from a tools/list answer.
func withoutRemoteUnavailable(tools []map[string]any) []map[string]any {
	out := make([]map[string]any, 0, len(tools))
	for _, t := range tools {
		if name, ok := t["name"].(string); ok {
			if _, blocked := remoteUnavailable[name]; blocked {
				continue
			}
		}
		out = append(out, t)
	}
	return out
}

// RemoteMember is the lead's grant for a remote team member: who it is in the team and where it works.
type RemoteMember struct {
	Member  string // its name in the team: "<agent>@<node>"
	Agent   string // the node's agent
	Node    string // the node it works on (the authenticated device name)
	Team    string // the team it is a member of
	Task    string // its task ID on the lead's bus (presence, the team's member record)
	Tier    Tier   // its tier in the team
	Repo    string // the repository name
	Branch  string // its branch, pushed to origin
	Job     string // the job that runs it on its node
	Wake    bool   // a wake request waits for its node (until its task runs)
	WakeNow bool   // a request to resume its paused task now (agent wake) waits for its node (until its task runs)
	Out     bool   // put out of its team: its node was away longer than the grace (it may come back)
	Paused  bool   // its task on the node is paused (a provider's limit, the network): reported to the lead once
}

// leadMCPBody is one MCP message of a remote member, relayed by its node.
type leadMCPBody struct {
	Member  string          // the member it comes from: "<agent>@<node>"
	Message json.RawMessage // one JSON-RPC message, as the member's CLI sent it
}

// remoteMemberFn hands a remote member's change to the agent script on the lead — `agent _remote_member <proxy>
// <status> <branch> <note>`, which keeps the proxy task's status and runs the team's logic (team_after_run) for it.
// A variable so that tests record the calls instead.
var remoteMemberFn = spawnRemoteMember

// spawnRemoteMember runs `agent _remote_member` for a member's proxy task, detached, as wakes are.
func spawnRemoteMember(t *Team, task, status, branch, note string) {
	logInfo(LogRemoteMemberEvent, "team", t.ID, "task", task, "status", status)
	spawnDetached(t.Home, t.WakeCmd, "_remote_member", task, status, branch, note)
}

// grantMember records a remote member's grant (replacing one of the same name).
func (a *leadAPI) grantMember(m RemoteMember) *LeadMemberError {
	agent, node, ok := strings.Cut(m.Member, "@")
	if !ok || agent != m.Agent || node != m.Node || !validJobIdentifier(agent) || !validJobIdentifier(node) ||
		!validJobIdentifier(m.Team) || !validJobIdentifier(m.Task) || !m.Tier.Valid() {
		return opErr(LeadMemberInvalidMember, "a remote member is <agent>@<node>, with a valid team, task and tier: %q", m.Member)
	}
	return leadTransaction(a, LeadMemberStorage, "", func(r *leadRegistry, _ Timestamp) *LeadMemberError {
		r.members[m.Member] = m
		return nil
	})
}

// revokeMember removes a remote member's grant.
func (a *leadAPI) revokeMember(member string) *LeadMemberError {
	return leadTransaction(a, LeadMemberStorage, "", func(r *leadRegistry, _ Timestamp) *LeadMemberError {
		if _, ok := r.members[member]; !ok {
			return opErr(LeadMemberNoSuchMember, "no remote member %q", member)
		}
		delete(r.members, member)
		return nil
	})
}

// mcp runs one MCP message of a remote member on the lead's bus and returns the JSON-RPC reply (nil for a
// notification).
func (a *leadAPI) mcp(node string, body leadMCPBody) (json.RawMessage, *LeadMCPError) {
	var probe struct {
		Method string `json:"method"` // the JSON-RPC method, for the log
	}
	if body.Member == "" || len(body.Message) == 0 || json.Unmarshal(body.Message, &probe) != nil {
		return nil, opErr(LeadMCPMalformedBody, "a member name and one JSON-RPC message are needed")
	}
	reg, serr := a.bus.loadJobs() // read only: grants change in their own transactions
	if serr != nil {
		return nil, fromStorage(LeadMCPStorage, serr)
	}
	g, ok := reg.members[body.Member]
	if !ok {
		return nil, opErr(LeadMCPNotAMember, "no remote member %q", body.Member)
	}
	if g.Node != node {
		return nil, opErr(LeadMCPUnauthorized, "member %q works on %q, not on %q", body.Member, g.Node, node)
	}
	var out bytes.Buffer
	s := &Server{bus: a.bus, me: g.Member, out: json.NewEncoder(&out), role: RoleAgent, team: g.Team, tier: g.Tier,
		remote: g.Node, home: agentsHome(),
		info: Presence{Agent: g.Member, Task: g.Task, Repo: g.Repo, Branch: g.Branch, Worktree: "remote:" + g.Node}}
	s.handle(body.Message)
	logInfo(LogLeadMCPCall, "member", g.Member, "node", node, "method", probe.Method)
	reply := bytes.TrimSpace(out.Bytes())
	if len(reply) == 0 {
		return nil, nil // a notification: no reply
	}
	return json.RawMessage(reply), nil
}

// wakeMember asks the member's node to wake it: the request waits in the node's heartbeat replies until the member's
// task runs again.
func (a *leadAPI) wakeMember(member string) *LeadMemberError {
	return leadTransaction(a, LeadMemberStorage, "", func(r *leadRegistry, _ Timestamp) *LeadMemberError {
		m, ok := r.members[member]
		if !ok {
			return opErr(LeadMemberNoSuchMember, "no remote member %q", member)
		}
		m.Wake = true
		r.members[member] = m
		return nil
	})
}

// wakeMemberNow asks the member's node to resume its paused task now (agent wake on its proxy task): the request
// waits in the node's heartbeat replies (WakesNow) until the member is reported at work again.
func (a *leadAPI) wakeMemberNow(member string) *LeadMemberError {
	return leadTransaction(a, LeadMemberStorage, "", func(r *leadRegistry, _ Timestamp) *LeadMemberError {
		m, ok := r.members[member]
		if !ok {
			return opErr(LeadMemberNoSuchMember, "no remote member %q", member)
		}
		m.WakeNow = true
		r.members[member] = m
		return nil
	})
}

// wakesNowFor lists the jobs of the node's members whose paused tasks its human resumes now.
func (a *leadAPI) wakesNowFor(node string) []string {
	reg, serr := a.bus.loadJobs()
	if serr != nil {
		warnSideEffect("list the node's resumes", serr)
		return nil // unreadable now: the next heartbeat asks again
	}
	var jobs []string
	for _, m := range reg.members {
		if m.Node == node && m.WakeNow {
			jobs = append(jobs, m.Job)
		}
	}
	sort.Strings(jobs)
	return jobs
}

// awake reports whether a task stage means its member is at work (a wake request has been carried out).
func awake(stage string) bool {
	switch stage {
	case "starting", "running", "investigating", "waiting":
		return true
	}
	return false
}

// memberActivity applies what a node says its remote members are doing (the stages of their jobs' tasks) to their
// teams — a member going to sleep goes through the same logic as a local one — and returns the jobs whose members
// the node must wake.
func (a *leadAPI) memberActivity(node string, act []JobActivity) ([]string, *LeadHeartbeatError) {
	reg, serr := a.bus.loadJobs()
	if serr != nil {
		return nil, fromStorage(LeadHeartbeatStorage, serr)
	}
	stages := make(map[string]string, len(act)) // job → its task's stage on the node
	notes := make(map[string]string, len(act))  // job → its latest activity line (why it paused)
	for _, x := range act {
		stages[x.ID] = x.Stage
		if n := len(x.Lines); n > 0 {
			notes[x.ID] = trunc(strings.TrimSpace(x.Lines[n-1]), 300)
		}
	}
	pauses := map[string]bool{} // members whose pause began (true) or ended (false)
	var resumed []string        // members whose paused tasks were resumed on request (WakeNow done)
	names := make([]string, 0, len(reg.members))
	for name := range reg.members {
		names = append(names, name)
	}
	sort.Strings(names)
	var wakes, carried []string
	for _, name := range names {
		m := reg.members[name]
		if m.Node != node || m.Job == "" {
			continue
		}
		stage, reported := stages[m.Job]
		if reported && (m.Out || (stage == "interrupted" && !m.Wake)) { // back after a restart or an absence
			if err := a.memberBack(m); err != nil {
				return nil, err
			}
			m.Out, m.Wake = false, true
		}
		if reported {
			pause := 0
			if stage == "paused" && !m.Paused {
				pause = 1
			} else if m.Paused && awake(stage) {
				pause = -1
			}
			if err := a.applyMemberStage(m, stage, pause, notes[m.Job]); err != nil {
				return nil, err
			}
			if pause != 0 {
				pauses[name] = pause > 0
			}
		}
		if m.Wake {
			if reported && awake(stage) {
				carried = append(carried, name)
			} else {
				wakes = append(wakes, m.Job)
			}
		}
		if m.WakeNow && reported && awake(stage) { // resumed: the request is done
			resumed = append(resumed, name)
		}
	}
	if len(resumed) > 0 {
		if err := leadTransaction(a, LeadHeartbeatStorage, "", func(r *leadRegistry, _ Timestamp) *LeadHeartbeatError {
			for _, name := range resumed {
				if m, ok := r.members[name]; ok {
					m.WakeNow = false
					r.members[name] = m
				}
			}
			return nil
		}); err != nil {
			return nil, err
		}
	}
	if len(pauses) > 0 {
		if err := leadTransaction(a, LeadHeartbeatStorage, "", func(r *leadRegistry, _ Timestamp) *LeadHeartbeatError {
			for name, p := range pauses {
				if m, ok := r.members[name]; ok {
					m.Paused = p
					r.members[name] = m
				}
			}
			return nil
		}); err != nil {
			return nil, err
		}
	}
	if len(carried) > 0 {
		if err := leadTransaction(a, LeadHeartbeatStorage, "", func(r *leadRegistry, _ Timestamp) *LeadHeartbeatError {
			for _, name := range carried {
				if m, ok := r.members[name]; ok {
					m.Wake = false
					r.members[name] = m
				}
			}
			return nil
		}); err != nil {
			return nil, err
		}
	}
	return wakes, nil
}

// applyMemberStage brings a remote member's state in its team in line with its task's stage on the node: asleep
// (through setMemberState: a waiting message wakes it) or at work. Other stages leave it as it is. A pause that began
// (pause > 0, with its reason) or ended (pause < 0) on the node is handed to the proxy task too.
func (a *leadAPI) applyMemberStage(m RemoteMember, stage string, pause int, note string) *LeadHeartbeatError {
	var team Team      // the member's team, for the hand-over after the commit
	var task string    // its task in the team\'s record (the proxy task), as wakes use it
	var changed string // "sleeping" or "running" when the member's state changed
	err := transact(a.bus, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
		t := st.Teams[m.Team]
		if t == nil || t.Members[m.Member] == nil || t.Done {
			return nil // the team is gone or finished: nothing to keep in line
		}
		team, task = *t, t.Members[m.Member].Task
		cur := t.Members[m.Member].State
		switch {
		case stage == "sleeping" && cur == MemberActive:
			changed = "sleeping"
			_, err := setMemberState(a.bus, st, m.Team, m.Member, MemberSleeping)
			return err
		case awake(stage) && (cur == MemberSleeping || cur == MemberWaking):
			changed = "running"
			_, err := setMemberState(a.bus, st, m.Team, m.Member, MemberActive)
			return err
		}
		return nil
	})
	if err != nil {
		return opErr(LeadHeartbeatStorage, "cannot keep remote member %s in line: %v", m.Member, err)
	}
	switch {
	case pause > 0 && task != "": // paused on its node: the proxy task and the human hear why, once
		remoteMemberFn(&team, task, "paused", m.Branch, note)
		return nil
	case pause < 0 && changed == "" && task != "": // back at work after its pause
		changed = "running"
	}
	if changed != "" { // the proxy task's status and the team's checks (stuck, all finished) follow
		remoteMemberFn(&team, task, changed, m.Branch, "")
	}
	return nil
}

// memberJobsChanged records what the lead learnt about member jobs in their teams: a reported branch, and the end
// of a job — the member is done and its coordinator is told.
func (a *leadAPI) memberJobsChanged(ids []string) {
	reg, serr := a.bus.loadJobs()
	if serr != nil {
		warnSideEffect("read member jobs", serr)
		return
	}
	for _, id := range ids {
		j := reg.jobs[id]
		if j == nil || j.Request.Team == "" {
			continue
		}
		job := *j
		var team Team // the member's team, for the hand-over after the commit
		var task string
		ended := false
		err := transact(a.bus, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
			t := st.Teams[job.Request.Team]
			if t == nil || t.Members[job.Request.Member] == nil {
				return nil
			}
			m := t.Members[job.Request.Member]
			if job.Branch != "" && m.Branch == "" {
				m.Branch = job.Branch
			}
			if !isTerminalJob(job.State) || m.State == MemberDone {
				return nil
			}
			team, task, ended = *t, m.Task, true
			_, err := setMemberState(a.bus, st, job.Request.Team, job.Request.Member, MemberDone)
			return err
		})
		if err != nil {
			warnSideEffect("update a remote member's team", err)
			continue
		}
		if ended { // the team's logic tells the coordinator (member_finished) or puts the member out (member_out)
			remoteMemberFn(&team, task, string(job.State), job.Branch, trunc(job.Result, 300))
		}
	}
}

// checkAbsentMembers puts out of their teams the remote members whose nodes have been away (no heartbeat) longer
// than grace — through the team's logic, as a failed member — once each; they may come back (memberActivity).
func (a *leadAPI) checkAbsentMembers(now time.Time, grace time.Duration) *LeadHeartbeatError {
	if grace <= 0 {
		return nil // no grace configured: never put out for an absence
	}
	type absence struct {
		m     RemoteMember // the member put out
		since time.Time    // its node's last heartbeat
	}
	var outs []absence
	err := leadTransaction(a, LeadHeartbeatStorage, "", func(r *leadRegistry, _ Timestamp) *LeadHeartbeatError {
		for name, m := range r.members {
			n, seen := r.nodes[m.Node]
			if m.Out || !seen || now.Sub(n.LastSeen.Time) <= grace {
				continue
			}
			m.Out = true
			r.members[name] = m
			outs = append(outs, absence{m, n.LastSeen.Time})
		}
		return nil
	})
	if err != nil {
		return err
	}
	if len(outs) == 0 {
		return nil
	}
	st, serr := a.bus.load()
	if serr != nil {
		return fromStorage(LeadHeartbeatStorage, serr)
	}
	for _, o := range outs {
		t := st.Teams[o.m.Team]
		if t == nil || t.Done || t.Members[o.m.Member] == nil {
			continue
		}
		team := *t
		remoteMemberFn(&team, t.Members[o.m.Member].Task, "out", o.m.Branch,
			fmt.Sprintf("its node %s has been away since %s (longer than %s)", o.m.Node, o.since.Format("15:04"), grace))
	}
	return nil
}

// memberBack takes a remote member back after its node's restart or absence: within the grace its items are still
// its own; after being put out it rejoins as an active member, its items having gone to others. Either way it is
// told so and woken (the wake rides on its node's heartbeat replies).
func (a *leadAPI) memberBack(m RemoteMember) *LeadHeartbeatError {
	if err := leadTransaction(a, LeadHeartbeatStorage, "", func(r *leadRegistry, _ Timestamp) *LeadHeartbeatError {
		if cur, ok := r.members[m.Member]; ok {
			cur.Out, cur.Wake = false, true
			r.members[m.Member] = cur
		}
		return nil
	}); err != nil {
		return err
	}
	var team Team   // the member's team, for the hand-over after the commit
	var task string // its proxy task
	err := transact(a.bus, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
		t := st.Teams[m.Team]
		if t == nil || t.Done || t.Members[m.Member] == nil {
			return nil // its team is gone or finished: the team's end stops the member's job
		}
		mine, released := memberItems(t, m.Member)
		var body string
		if m.Out {
			if _, err := setMemberState(a.bus, st, m.Team, m.Member, MemberActive); err != nil {
				return err
			}
			team, task = *t, t.Members[m.Member].Task
			body = fmt.Sprintf("TEAM: your node %s was away too long and you were out of the team; your board items went back to the board (%s). You are back as an available member: check the board and take new work.", m.Node, released)
		} else {
			body = fmt.Sprintf("TEAM: your node %s restarted under your last run. Your board items are still yours (%s): finish them.", m.Node, mine)
		}
		_, serr := a.bus.post(st, "agentbus", m.Member, EventScript{Notice: NoticeMemberBack, Body: body,
			Fields: map[string]string{"agent": m.Member, "team": m.Team, "node": m.Node}})
		return fromStorage(CmdTeamSetStorage, serr)
	})
	if err != nil {
		return opErr(LeadHeartbeatStorage, "cannot take remote member %s back: %v", m.Member, err)
	}
	if task != "" { // its proxy task runs again (it was failed when the member was put out)
		remoteMemberFn(&team, task, "running", m.Branch, "")
	}
	return nil
}

// memberItems lists a member's board items: those it holds, and those it held before they went back to the board
// (with who holds them now), as text for a message.
func memberItems(t *Team, member string) (mine, released string) {
	var held, gone []string
	ids := make([]string, 0, len(t.Board))
	for id := range t.Board {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		it := t.Board[id]
		switch {
		case it.Assignee == member && it.Status == BoardClaimed:
			held = append(held, id)
		case it.Previous == member && it.Assignee != "" && it.Assignee != member:
			gone = append(gone, id+" now "+it.Assignee+"'s")
		case it.Previous == member:
			gone = append(gone, id+" open")
		}
	}
	mine, released = "none", "none"
	if len(held) > 0 {
		mine = strings.Join(held, ", ")
	}
	if len(gone) > 0 {
		released = strings.Join(gone, ", ")
	}
	return mine, released
}

// relayMCP is a remote member's MCP server on its node: every JSON-RPC line from the CLI goes to the lead, every
// reply back; an unreachable lead is told to the agent.
func relayMCP(cfg nodeRequestConfig, member string, in io.Reader, out io.Writer) *NodeClientError {
	cfg.path, cfg.get = "/v1/mcp", false
	reader := bufio.NewReader(in)
	for {
		line, rerr := reader.ReadBytes('\n')
		if len(bytes.TrimSpace(line)) > 0 {
			if werr := relayLine(cfg, member, bytes.TrimSpace(line), out); werr != nil {
				return werr
			}
		}
		if rerr == io.EOF {
			return nil
		}
		if rerr != nil {
			return opErr(NodeClientInvalidArgument, "cannot read the CLI's MCP messages: %v", rerr)
		}
	}
}

// relayLine sends one JSON-RPC message to the lead and writes its reply; when the lead cannot be reached or refuses,
// the agent is told (a tool error for a tool call, a JSON-RPC error for anything else, nothing for a notification).
func relayLine(cfg nodeRequestConfig, member string, line []byte, out io.Writer) *NodeClientError {
	var req struct {
		ID     json.RawMessage `json:"id"`     // the request's ID (absent: a notification)
		Method string          `json:"method"` // the JSON-RPC method
	}
	if err := json.Unmarshal(line, &req); err != nil {
		return writeRPC(out, map[string]any{"jsonrpc": "2.0", "id": nil, "error": map[string]any{"code": rpcParseError, "message": "parse error: " + err.Error()}})
	}
	body, err := json.Marshal(leadMCPBody{Member: member, Message: line})
	if err != nil {
		return opErr(NodeClientInvalidArgument, "cannot encode the message: %v", err)
	}
	reply, cerr := nodeRequest(cfg, bytes.NewReader(body))
	if cerr == nil {
		var wrapped struct {
			Message json.RawMessage // the lead's JSON-RPC reply; null for a notification
		}
		if err := json.Unmarshal(reply, &wrapped); err != nil {
			cerr = opErr(NodeClientMalformedReply, "the lead's reply is not {Message}: %v", err)
		} else {
			if len(wrapped.Message) == 0 || string(wrapped.Message) == "null" {
				return nil
			}
			if _, werr := out.Write(append(append([]byte{}, wrapped.Message...), '\n')); werr != nil {
				return opErr(NodeClientInvalidArgument, "cannot write to the CLI: %v", werr)
			}
			return nil
		}
	}
	logWarn(LogRelayLeadFailed, "member", member, "method", req.Method, "cause", cerr.Cause.String(), "error", cerr.Message)
	if len(req.ID) == 0 || string(req.ID) == "null" {
		return nil // a notification: nothing to answer
	}
	why := "the lead is unreachable"
	if cerr.Cause == NodeClientRejected {
		why = "the lead refused this member"
	}
	text := fmt.Sprintf("ERROR: %s (%s). End your turn with STATUS: SLEEPING — you are woken when the lead is back.", why, cerr.Message)
	if req.Method == "tools/call" {
		return writeRPC(out, map[string]any{"jsonrpc": "2.0", "id": req.ID,
			"result": map[string]any{"content": []map[string]any{{"type": "text", "text": text}}, "isError": true}})
	}
	return writeRPC(out, map[string]any{"jsonrpc": "2.0", "id": req.ID, "error": map[string]any{"code": -32000, "message": text}})
}

// writeRPC writes one JSON-RPC message line to the CLI.
func writeRPC(out io.Writer, msg map[string]any) *NodeClientError {
	data, err := json.Marshal(msg)
	if err != nil {
		return opErr(NodeClientInvalidArgument, "cannot encode a reply: %v", err)
	}
	if _, err := out.Write(append(data, '\n')); err != nil {
		return opErr(NodeClientInvalidArgument, "cannot write to the CLI: %v", err)
	}
	return nil
}

// cmdLeadMember — lead-member: a remote team member's grant on the lead (the agent script, starting or ending a team
// with remote members, and waking one).
//
//	agentbus lead-member --dir DIR --add --member a@node --agent a --node node --team T --task P --tier normal \
//	    [--repo R] [--branch B] --job J
//	agentbus lead-member --dir DIR --remove --member a@node
//	agentbus lead-member --dir DIR --wake --member a@node
//	agentbus lead-member --dir DIR --wake-now --member a@node
func cmdLeadMember(args []string) {
	fs := newFlagSet("lead-member")
	dir := fs.String("dir", defaultDir(), "state directory")
	add := fs.Bool("add", false, "grant a remote member")
	remove := fs.Bool("remove", false, "revoke a remote member's grant")
	wake := fs.Bool("wake", false, "ask the member's node to wake it")
	wakeNow := fs.Bool("wake-now", false, "ask the member's node to resume its paused task now")
	m := RemoteMember{}
	fs.StringVar(&m.Member, "member", "", "the member: <agent>@<node>")
	fs.StringVar(&m.Agent, "agent", "", "the node's agent")
	fs.StringVar(&m.Node, "node", "", "the node")
	fs.StringVar(&m.Team, "team", "", "the team")
	fs.StringVar(&m.Task, "task", "", "its proxy task on the lead")
	tier := fs.String("tier", string(TierNormal), "normal | worker")
	fs.StringVar(&m.Repo, "repo", "", "the repository name")
	fs.StringVar(&m.Branch, "branch", "", "its branch, if known")
	fs.StringVar(&m.Job, "job", "", "the job that runs it on its node")
	parseFlags(fs, args)
	m.Tier = Tier(*tier)
	a := &leadAPI{bus: &Bus{dir: *dir}}
	var err *LeadMemberError
	switch {
	case *add && !*remove && !*wake && !*wakeNow:
		err = a.grantMember(m)
	case *remove && !*add && !*wake && !*wakeNow:
		err = a.revokeMember(m.Member)
	case *wake && !*add && !*remove && !*wakeNow:
		err = a.wakeMember(m.Member)
	case *wakeNow && !*add && !*remove && !*wake:
		err = a.wakeMemberNow(m.Member)
	default:
		err = opErr(LeadMemberInvalidMember, "give exactly one of --add, --remove, --wake, --wake-now")
	}
	exitOn(err.asError())
}

// The doorbell: a node keeps GET /v1/events open; the lead rings it when something new waits for it — a wake of one
// of its remote members, a stop of one of its jobs, a queued job — and the node polls at once. A ring carries no
// data: the node's ordinary heartbeat fetches what waits, so a lost ring or a broken stream loses nothing.
const (
	doorbellPoll      = 250 * time.Millisecond // how often a stream looks at what waits for its node
	doorbellKeepAlive = 20 * time.Second       // a comment line that keeps an idle stream open
)

// pendingFor describes what waits for a node ("" = nothing): the wakes of its members, the stops of its jobs, the
// number of queued jobs, and pending answers and kicks. A ring follows a change of it, so that an ordinary heartbeat rings nothing.
func (a *leadAPI) pendingFor(node string) string {
	reg, serr := a.bus.loadJobs()
	if serr != nil {
		return "" // unreadable now: the node's own heartbeat still comes
	}
	var wakes, stops, answers, kicks []string
	queued := 0
	for _, m := range reg.members {
		if m.Node == node && (m.Wake || m.WakeNow) {
			wakes = append(wakes, m.Job)
		}
	}
	for id, j := range reg.jobs {
		if j.Node == node && !isTerminalJob(j.State) {
			for _, ans := range j.PendingAnswers {
				answers = append(answers, fmt.Sprintf("%s:%s", id, ans.SHA256))
			}
			for _, kick := range j.PendingKicks {
				kicks = append(kicks, fmt.Sprintf("%s:%s", id, kick.SHA256))
			}
		}
		switch {
		case j.Node == node && j.StopRequested:
			stops = append(stops, id)
		case j.State == JobQueued:
			queued++
		}
	}
	if len(wakes) == 0 && len(stops) == 0 && queued == 0 && len(answers) == 0 && len(kicks) == 0 {
		return ""
	}
	sort.Strings(wakes)
	sort.Strings(stops)
	sort.Strings(answers)
	sort.Strings(kicks)
	return fmt.Sprintf("w:%s s:%s q:%d a:%s k:%s", strings.Join(wakes, ","), strings.Join(stops, ","), queued, strings.Join(answers, ","), strings.Join(kicks, ","))
}

// doorbell serves a node's event stream: "event: ring" when what waits for it changes, a comment now and then.
func (a *leadAPI) doorbell(w http.ResponseWriter, r *http.Request, node string) {
	rc := http.NewResponseController(w)
	if err := rc.SetWriteDeadline(time.Time{}); err != nil { // the stream outlives the server's write timeout
		logWarn(LogLeadHTTPError, "operation", "lead/events", "node", node, "error", err.Error())
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	send := func(s string) bool {
		if _, err := io.WriteString(w, s); err != nil {
			return false
		}
		return rc.Flush() == nil
	}
	if !send(": connected\n\n") {
		return
	}
	tick, keep := time.NewTicker(doorbellPoll), time.NewTicker(doorbellKeepAlive)
	defer tick.Stop()
	defer keep.Stop()
	last := ""
	for {
		select {
		case <-r.Context().Done():
			return
		case <-keep.C:
			if !send(": keepalive\n\n") {
				return
			}
		case <-tick.C:
			now := a.pendingFor(node)
			if now == last {
				continue
			}
			last = now
			if now == "" {
				continue
			}
			logInfo(LogDoorbellRing, "node", node, "pending", now)
			if !send("event: ring\ndata: {}\n\n") {
				return
			}
		}
	}
}

// cmdNodeDoorbell — node-doorbell: the node's side of the doorbell; prints "ring" for every ring of the lead and
// reconnects after a broken stream; ends when its output is closed (the node daemon is gone).
//
//	agentbus node-doorbell --url http://LEAD:PORT [--source IP]
func cmdNodeDoorbell(args []string) {
	fs := newFlagSet("node-doorbell")
	base := fs.String("url", "", "the lead's URL")
	source := fs.String("source", "", "an optional local source IP (isolated installations, as node-request)")
	parseFlags(fs, args)
	u, err := url.Parse(*base)
	if err != nil || u.Scheme != "http" || u.Hostname() == "" || u.User != nil || (u.Path != "" && u.Path != "/") ||
		(*source != "" && net.ParseIP(*source) == nil) {
		exitOn(opErr(NodeClientInvalidArgument, "--url must be a plain HTTP lead origin; --source an IP literal").asError())
	}
	for {
		closed, lerr := listenDoorbell(strings.TrimSuffix(*base, "/"), *source, os.Stdout)
		if closed {
			return
		}
		if lerr != nil {
			logWarn(LogDoorbellLost, "cause", lerr.Cause.String(), "error", lerr.Message)
		}
		time.Sleep(2 * time.Second)
	}
}

// listenDoorbell follows one event stream and writes "ring" lines; closed reports that the output is gone.
func listenDoorbell(base, source string, out io.Writer) (closed bool, err *NodeClientError) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	if source != "" {
		dialer.LocalAddr = &net.TCPAddr{IP: net.ParseIP(source)}
	}
	transport := &http.Transport{DialContext: dialer.DialContext, ResponseHeaderTimeout: 5 * time.Second}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }} // http.Client requires the plain interface error.
	resp, gerr := client.Get(base + "/v1/events")
	if gerr != nil {
		return false, opErr(NodeClientUnreachable, "cannot reach the lead's doorbell: %v", gerr)
	}
	defer func() {
		if cerr := resp.Body.Close(); cerr != nil {
			logWarn(LogDoorbellLost, "error", cerr.Error())
		}
	}()
	if resp.StatusCode != http.StatusOK {
		return false, opErr(NodeClientRejected, "the lead's doorbell answered HTTP %d", resp.StatusCode)
	}
	sc := bufio.NewScanner(resp.Body)
	for sc.Scan() {
		if sc.Text() != "event: ring" {
			continue
		}
		if _, werr := io.WriteString(out, "ring\n"); werr != nil {
			return true, nil
		}
	}
	if serr := sc.Err(); serr != nil {
		return false, opErr(NodeClientUnreachable, "the doorbell stream broke: %v", serr)
	}
	return false, opErr(NodeClientUnreachable, "the lead closed the doorbell stream")
}
