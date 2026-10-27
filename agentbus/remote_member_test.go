package main

// Tests of remote team members (written first, docs/tdd.md): a remote member's tools run on the lead's bus with
// the identity of its grant; the request must come from the node it works on; what a remote member cannot do is
// refused with a clear message; the relay on the node tells the agent when the lead is unreachable; the grant
// survives a restart of the lead.

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// remoteMember is the member of team T that works on rpi5-x.
var remoteMember = RemoteMember{Member: "gemini-2@rpi5-x", Agent: "gemini-2", Node: "rpi5-x", Team: "T", Task: "b",
	Tier: TierNormal, Repo: "repo", Branch: "agents/gemini-2/x"}

// remoteSetup is a lead with team T: lead (local) and gemini-2@rpi5-x (remote, granted).
func remoteSetup(t *testing.T) (*Bus, *leadAPI) {
	t.Helper()
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "gemini-2@rpi5-x": {"normal", "b", "active"}})
	a := &leadAPI{bus: b, ttl: time.Minute}
	if err := a.grantMember(remoteMember); err != nil {
		t.Fatal(err)
	}
	return b, a
}

// rpc is one JSON-RPC request line.
func rpc(id int, method string, params any) json.RawMessage {
	line, err := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": id, "method": method, "params": params})
	if err != nil {
		panic(err)
	}
	return line
}

// toolCall is a tools/call request line.
func toolCall(id int, name string, args map[string]any) json.RawMessage {
	return rpc(id, "tools/call", map[string]any{"name": name, "arguments": args})
}

// relayed runs one message of the remote member as if from its node and fails the test on a refusal.
func relayed(t *testing.T, a *leadAPI, msg json.RawMessage) string {
	t.Helper()
	reply, err := a.mcp(remoteMember.Node, leadMCPBody{Member: remoteMember.Member, Message: msg})
	if err != nil {
		t.Fatalf("relayed message refused: %v", err)
	}
	return string(reply)
}

func TestRemoteMemberToolsRunOnTheLeadsBus(t *testing.T) {
	b, a := remoteSetup(t)
	if r := relayed(t, a, toolCall(1, "send_message", map[string]any{"to": "lead", "text": "hello from the node"})); strings.Contains(r, `"isError":true`) || !strings.Contains(r, `"id":1`) {
		t.Fatalf("send_message: %s", r)
	}
	if r := relayed(t, a, toolCall(2, "board_add", map[string]any{"title": "parser on arm64", "details": "run it there", "level": "normal"})); strings.Contains(r, `"isError":true`) {
		t.Fatalf("board_add: %s", r)
	}
	lead := server(b, "lead", inTeam("T"))
	if out := chk(t).ok(toolResult(lead.read(0))); !strings.Contains(out, "gemini-2@rpi5-x") || !strings.Contains(out, "hello from the node") {
		t.Errorf("the lead does not see the remote member's message:\n%s", out)
	}
	if out := chk(t).ok(toolResult(lead.boardList())); !strings.Contains(out, "parser on arm64") {
		t.Errorf("the team's board lacks the remote member's item:\n%s", out)
	}
}

func TestRemoteMemberIsCheckedAgainstItsNode(t *testing.T) {
	_, a := remoteSetup(t)
	msg := toolCall(1, "board_list", nil)
	_, err := a.mcp("other-node", leadMCPBody{Member: remoteMember.Member, Message: msg})
	failsWith(t, LeadMCPUnauthorized)("another node", err)
	_, err = a.mcp("rpi5-x", leadMCPBody{Member: "nobody@rpi5-x", Message: msg})
	failsWith(t, LeadMCPNotAMember)("unknown member", err)
	_, err = a.mcp("rpi5-x", leadMCPBody{Member: remoteMember.Member, Message: json.RawMessage(`not json`)})
	failsWith(t, LeadMCPMalformedBody)("not JSON-RPC", err)
	if err := a.revokeMember(remoteMember.Member); err != nil {
		t.Fatal(err)
	}
	_, err = a.mcp("rpi5-x", leadMCPBody{Member: remoteMember.Member, Message: msg})
	failsWith(t, LeadMCPNotAMember)("revoked", err)
	failsWith(t, LeadMemberNoSuchMember)("revoke twice", a.revokeMember(remoteMember.Member))
	failsWith(t, LeadMemberInvalidMember)("name not agent@node", a.grantMember(RemoteMember{Member: "gemini-2", Agent: "gemini-2", Node: "rpi5-x", Team: "T", Task: "b", Tier: TierNormal}))
}

func TestRemoteMemberCannotReserveCoordinateDelegateOrWait(t *testing.T) {
	_, a := remoteSetup(t)
	for i, tool := range []string{"reserve", "release", "claim_lead", "handover_lead", "delegate", "wait_for_message"} {
		r := relayed(t, a, toolCall(i+1, tool, map[string]any{}))
		if !strings.Contains(r, `"isError":true`) || !strings.Contains(r, "not available to a remote member") {
			t.Errorf("%s: %s", tool, r)
		}
	}
	list := relayed(t, a, rpc(9, "tools/list", map[string]any{}))
	for _, hidden := range []string{`"reserve"`, `"claim_lead"`, `"delegate"`, `"wait_for_message"`} {
		if strings.Contains(list, hidden) {
			t.Errorf("tools/list offers %s to a remote member", hidden)
		}
	}
	if !strings.Contains(list, `"board_add"`) || !strings.Contains(list, `"send_message"`) {
		t.Errorf("tools/list lacks the team's tools: %.300s", list)
	}
}

func TestRemoteMemberGrantSurvivesARestartOfTheLead(t *testing.T) {
	b, _ := remoteSetup(t)
	again := &leadAPI{bus: b, ttl: time.Minute} // a new process: everything it knows comes from jobs.json
	relayed(t, again, toolCall(1, "board_list", nil))
}

func TestRelayTellsTheMemberWhenTheLeadIsUnreachable(t *testing.T) {
	var out bytes.Buffer
	in := strings.NewReader(string(rpc(1, "initialize", map[string]any{})) + "\n" + string(toolCall(2, "board_list", nil)) + "\n")
	if err := relayMCP(nodeRequestConfig{url: "http://127.0.0.1:1"}, remoteMember.Member, in, &out); err != nil {
		t.Fatalf("the relay itself must not fail when the lead is unreachable: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(out.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("want a reply per request, got %q", out.String())
	}
	if !strings.Contains(lines[0], `"error"`) || !strings.Contains(lines[0], `"id":1`) {
		t.Errorf("initialize without a lead: %s", lines[0])
	}
	if !strings.Contains(lines[1], `"isError":true`) || !strings.Contains(lines[1], "lead is unreachable") || !strings.Contains(lines[1], "STATUS: SLEEPING") {
		t.Errorf("a tool call without a lead: %s", lines[1])
	}
}

func TestLeadMCPRouteChecksTheNode(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "gemini-2@node-a": {"normal", "b", "active"}, "gemini-2@rpi5-x": {"normal", "c", "active"}})
	a := &leadAPI{bus: b, ttl: time.Minute}
	for _, m := range []RemoteMember{
		{Member: "gemini-2@node-a", Agent: "gemini-2", Node: "node-a", Team: "T", Task: "b", Tier: TierNormal},
		{Member: "gemini-2@rpi5-x", Agent: "gemini-2", Node: "rpi5-x", Team: "T", Task: "c", Tier: TierNormal},
	} {
		if err := a.grantMember(m); err != nil {
			t.Fatal(err)
		}
	}
	ts := httptest.NewServer(newLeadHandler(b, "node-a", nil, time.Minute)) // loopback is the lead's own node, node-a
	t.Cleanup(ts.Close)
	post := func(member string) int {
		body, err := json.Marshal(leadMCPBody{Member: member, Message: toolCall(1, "board_list", nil)})
		if err != nil {
			t.Fatal(err)
		}
		resp, err := http.Post(ts.URL+"/v1/mcp", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		if err := resp.Body.Close(); err != nil {
			t.Fatal(err)
		}
		return resp.StatusCode
	}
	if code := post("gemini-2@node-a"); code != http.StatusOK {
		t.Errorf("the member of node-a, from node-a: HTTP %d", code)
	}
	if code := post("gemini-2@rpi5-x"); code != http.StatusForbidden {
		t.Errorf("the member of rpi5-x, from node-a: HTTP %d, want 403", code)
	}
}
