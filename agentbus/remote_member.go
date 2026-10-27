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
	"strings"
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
	Member string // its name in the team: "<agent>@<node>"
	Agent  string // the node's agent
	Node   string // the node it works on (the authenticated device name)
	Team   string // the team it is a member of
	Task   string // its task ID on the lead's bus (presence, the team's member record)
	Tier   Tier   // its tier in the team
	Repo   string // the repository name
	Branch string // its branch, pushed to origin
	Job    string // the job that runs it on its node
	Wake   bool   // a wake request waits for its node (until its task runs)
}

// leadMCPBody is one MCP message of a remote member, relayed by its node.
type leadMCPBody struct {
	Member  string          // the member it comes from: "<agent>@<node>"
	Message json.RawMessage // one JSON-RPC message, as the member's CLI sent it
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
func (a *leadAPI) wakeMember(_ string) *LeadMemberError {
	return nil // not implemented yet: remote_team_test.go comes first
}

// memberActivity applies what a node says its remote members are doing (the stages of their jobs' tasks) to their
// teams — a member going to sleep goes through the same logic as a local one — and returns the jobs whose members
// the node must wake.
func (a *leadAPI) memberActivity(_ string, _ []JobActivity) ([]string, *LeadHeartbeatError) {
	return nil, nil // not implemented yet
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
