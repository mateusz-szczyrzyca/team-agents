// Remote team members: a member that works on a node is a member of a team on the lead, the team's only bus. Its
// node relays its MCP messages to the lead (POST /v1/mcp); the lead runs them with the same code as a local
// member's (Server.handle), with the identity its grant gives — never one a request claims.
package main

import (
	"encoding/json"
	"io"
)

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
}

// leadMCPBody is one MCP message of a remote member, relayed by its node.
type leadMCPBody struct {
	Member  string          // the member it comes from: "<agent>@<node>"
	Message json.RawMessage // one JSON-RPC message, as the member's CLI sent it
}

// grantMember records a remote member's grant (replacing one of the same name).
func (a *leadAPI) grantMember(_ RemoteMember) *LeadMemberError {
	return nil // not implemented yet: remote_member_test.go comes first
}

// revokeMember removes a remote member's grant.
func (a *leadAPI) revokeMember(_ string) *LeadMemberError {
	return nil // not implemented yet
}

// mcp runs one MCP message of a remote member on the lead's bus and returns the JSON-RPC reply (nil for a
// notification).
func (a *leadAPI) mcp(_ string, _ leadMCPBody) (json.RawMessage, *LeadMCPError) {
	return nil, nil // not implemented yet
}

// relayMCP is a remote member's MCP server on its node: every JSON-RPC line from the CLI goes to the lead, every
// reply back; an unreachable lead is told to the agent.
func relayMCP(_ nodeRequestConfig, _ string, _ io.Reader, _ io.Writer) *NodeClientError {
	return nil // not implemented yet
}
