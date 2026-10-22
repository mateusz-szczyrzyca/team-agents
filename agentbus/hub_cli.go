// CLI operations on the hub execute shared bus subcommands against the hub's state.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

// hubCLIDispatcher returns a dispatcher function for allowlisted Hub CLI operations.
func hubCLIDispatcher(b *Bus, instances *HubInstanceRegistry) func(HubRequest) HubResponse {
	return func(req HubRequest) HubResponse {
		switch req.Op {
		case "send":
			return handleCLISend(b, req)
		case "team-get":
			return handleCLITeamGet(b, req)
		case "team-set":
			return handleCLITeamSet(b, req)
		case "team-done":
			return handleCLITeamDone(b, req)
		case "team-create":
			return handleCLITeamCreate(b, req)
		case "instances":
			return handleCLIInstances(b, instances, req)
		case "status":
			return handleCLIStatus(b, req)
		case "log":
			return handleCLILog(b, req)
		default:
			return HubResponse{
				RequestID: req.RequestID,
				Error:     hubFailure(opErr(HubCallUnknownOperation, "unsupported CLI operation %q", req.Op)),
			}
		}
	}
}

// handleCLISend posts a message to the bus via transact.
func handleCLISend(b *Bus, req HubRequest) HubResponse {
	text, _ := req.Args["text"].(string)
	text = strings.TrimSpace(text)
	if text == "" {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "send", Cause: CmdSendMissingText.String(), Message: missing("message text")},
		}
	}
	from, _ := req.Args["from"].(string)
	if from == "" {
		from = req.Caller.Agent
		if req.Caller.Instance != "" {
			from = req.Caller.Agent + "@" + req.Caller.Instance
		}
	}
	to, _ := req.Args["to"].(string)
	if to == "" {
		to = "all"
	}
	noticeStr, _ := req.Args["notice"].(string)
	var ev busEvent = EventChat{Body: text}
	if noticeStr != "" {
		n, verr := parseEnum[ScriptNotice](noticeStr, "script notice")
		if verr != nil {
			return HubResponse{
				RequestID: req.RequestID,
				Error:     &HubWireError{Op: "send", Cause: CmdSendInvalidNotice.String(), Message: verr.Error()},
			}
		}
		fields := make(fieldFlags)
		if rawMap, ok := req.Args["fields"].(map[string]any); ok {
			for k, v := range rawMap {
				if s, ok := v.(string); ok {
					fields[k] = s
				}
			}
		}
		ev = EventScript{Notice: n, Fields: fields, Body: text}
	}

	var out bytes.Buffer
	var postErr *CmdSendError
	serr := transact(b, CmdSendStorage, func(st *State) *CmdSendError {
		m, postSerr := b.post(st, from, to, ev)
		if postSerr != nil {
			return fromStorage(CmdSendStorage, postSerr)
		}
		fmt.Fprintln(&out, fmtMsg(m))
		return nil
	})
	if serr != nil {
		postErr = serr
	}
	if postErr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "send", Cause: postErr.Cause.String(), Message: postErr.Message},
		}
	}
	buf := out.Bytes()
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &buf,
	}
}

// handleCLITeamGet retrieves team state as JSON.
func handleCLITeamGet(b *Bus, req HubRequest) HubResponse {
	teamID, _ := req.Args["team"].(string)
	if teamID == "" {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-get", Cause: CmdTeamGetNoSuchTeam.String(), Message: missing("team name")},
		}
	}
	var out []byte
	serr := b.locked(func(st *State) {
		t := st.Teams[teamID]
		if t == nil {
			return
		}
		data, err := json.Marshal(t)
		if err == nil {
			out = append(data, '\n')
		}
	})
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-get", Cause: CmdTeamGetStorage.String(), Message: serr.Error()},
		}
	}
	if len(out) == 0 {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-get", Cause: CmdTeamGetNoSuchTeam.String(), Message: fmt.Sprintf("team %q not found", teamID)},
		}
	}
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &out,
	}
}

// handleCLITeamSet sets a member's state in a team.
func handleCLITeamSet(b *Bus, req HubRequest) HubResponse {
	teamID, _ := req.Args["team"].(string)
	agent, _ := req.Args["agent"].(string)
	stateStr, _ := req.Args["state"].(string)
	if teamID == "" || agent == "" || stateStr == "" {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-set", Cause: CmdTeamSetNoSuchMember.String(), Message: "missing required arguments"},
		}
	}
	stVal, verr := parseEnum[MemberState](stateStr, "member state")
	if verr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-set", Cause: CmdTeamSetInvalidState.String(), Message: verr.Error()},
		}
	}
	serr := transact(b, CmdTeamSetStorage, func(st *State) *CmdTeamSetError {
		t := st.Teams[teamID]
		if t == nil {
			return opErr(CmdTeamSetNoSuchMember, "team %q not found", teamID)
		}
		m := t.Members[agent]
		if m == nil {
			// Check if qualified name matches
			for k, mem := range t.Members {
				if mem.Agent == agent || strings.HasPrefix(k, agent+"@") {
					m = mem
					break
				}
			}
		}
		if m == nil {
			return opErr(CmdTeamSetNoSuchMember, "member %q not found in team %q", agent, teamID)
		}
		m.State = stVal
		return nil
	})
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-set", Cause: serr.Cause.String(), Message: serr.Message},
		}
	}
	empty := []byte{}
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &empty,
	}
}

// handleCLITeamDone marks a team as finished.
func handleCLITeamDone(b *Bus, req HubRequest) HubResponse {
	teamID, _ := req.Args["team"].(string)
	if teamID == "" {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-done", Cause: CmdTeamDoneStorage.String(), Message: missing("team name")},
		}
	}
	serr := transact(b, CmdTeamDoneStorage, func(st *State) *CmdTeamDoneError {
		t := st.Teams[teamID]
		if t == nil {
			return opErr(CmdTeamDoneStorage, "team %q not found", teamID)
		}
		t.Done = true
		return nil
	})
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-done", Cause: serr.Cause.String(), Message: serr.Message},
		}
	}
	empty := []byte{}
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &empty,
	}
}

// handleCLITeamCreate creates a team on the hub bus.
func handleCLITeamCreate(b *Bus, req HubRequest) HubResponse {
	var t Team
	if raw, ok := req.Args["team_json"].(string); ok {
		if err := json.Unmarshal([]byte(raw), &t); err != nil {
			return HubResponse{
				RequestID: req.RequestID,
				Error:     &HubWireError{Op: "team-create", Cause: CmdTeamCreateInvalidJSON.String(), Message: err.Error()},
			}
		}
	} else if rawMap, ok := req.Args["team"].(map[string]any); ok {
		data, _ := json.Marshal(rawMap)
		if err := json.Unmarshal(data, &t); err != nil {
			return HubResponse{
				RequestID: req.RequestID,
				Error:     &HubWireError{Op: "team-create", Cause: CmdTeamCreateInvalidJSON.String(), Message: err.Error()},
			}
		}
	} else {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-create", Cause: CmdTeamCreateMissingFields.String(), Message: "missing team specification"},
		}
	}
	serr := transact(b, CmdTeamCreateStorage, func(st *State) *CmdTeamCreateError {
		if st.Teams == nil {
			st.Teams = make(map[string]*Team)
		}
		if _, exists := st.Teams[t.ID]; exists {
			return opErr(CmdTeamCreateStorage, "team %q already exists", t.ID)
		}
		st.Teams[t.ID] = &t
		return nil
	})
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "team-create", Cause: serr.Cause.String(), Message: serr.Message},
		}
	}
	empty := []byte{}
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &empty,
	}
}

// handleCLIInstances lists the registered instances.
func handleCLIInstances(_ *Bus, instances *HubInstanceRegistry, req HubRequest) HubResponse {
	if instances == nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "instances", Cause: CmdInstancesStorage.String(), Message: "instances registry is not configured"},
		}
	}
	records, err := instances.load()
	if err != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "instances", Cause: CmdInstancesStorage.String(), Message: err.Error()},
		}
	}
	data, mErr := json.Marshal(records)
	if mErr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "instances", Cause: CmdInstancesStorage.String(), Message: err.Error()},
		}
	}
	out := append(data, '\n')
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &out,
	}
}

// handleCLIStatus prints the hub's bus status.
func handleCLIStatus(b *Bus, req HubRequest) HubResponse {
	var out bytes.Buffer
	serr := b.locked(func(st *State) {
		fmt.Fprintf(&out, "Bus: %s\n", b.dir)
		for k, t := range st.Teams {
			fmt.Fprintf(&out, "Team %s (repo %s, lead %s, done: %v)\n", k, t.Repo, t.Lead, t.Done)
		}
	})
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "status", Cause: CmdStatusStorage.String(), Message: serr.Error()},
		}
	}
	buf := out.Bytes()
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &buf,
	}
}

// handleCLILog prints the last N messages from the hub.
func handleCLILog(b *Bus, req HubRequest) HubResponse {
	msgs, serr := b.messages()
	if serr != nil {
		return HubResponse{
			RequestID: req.RequestID,
			Error:     &HubWireError{Op: "log", Cause: CmdLogStorage.String(), Message: serr.Error()},
		}
	}
	n := 30
	if rawN, ok := req.Args["n"].(float64); ok && rawN > 0 {
		n = int(rawN)
	}
	start := len(msgs) - n
	if start < 0 {
		start = 0
	}
	var out bytes.Buffer
	for _, m := range msgs[start:] {
		fmt.Fprintln(&out, fmtMsg(m))
	}
	buf := out.Bytes()
	return HubResponse{
		RequestID: req.RequestID,
		Stdout:    &buf,
	}
}
