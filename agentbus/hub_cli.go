// CLI operations on the hub execute shared bus subcommands against the hub's state.
package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
)

// hubCLIDispatcher returns a dispatcher function for allowlisted Hub CLI operations.
func hubCLIDispatcher(b *Bus, instances *HubInstanceRegistry) func(HubRequest) HubResponse {
	return func(req HubRequest) HubResponse {
		if !hubOperator(req.Caller) {
			return hubErrorResponse(req.RequestID, opErr(HubCallIdentity, "CLI operations require an authenticated instance operator"))
		}
		req.Args = qualifyHubArgs(req.Args, req.Caller.Instance)
		switch req.Op {
		case "command_enqueue":
			return handleCLIEnqueue(b, instances, req)
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
	var out []byte
	err := transact(b, CmdTeamGetStorage, func(st *State) *CmdTeamGetError {
		t := st.Teams[teamID]
		if t == nil || !teamHasInstance(t, req.Caller.Instance) {
			return opErr(CmdTeamGetNoSuchTeam, "no team %s on this instance", teamID)
		}
		public := *t
		public.Quiz, public.Consults, public.WakeCmd, public.Home = nil, nil, "", ""
		public.Members = make(map[string]*TeamMember, len(t.Members))
		for name, member := range t.Members {
			publicMember := *member
			publicMember.Worktree = ""
			public.Members[name] = &publicMember
		}
		data, marshalErr := json.Marshal(&public)
		if marshalErr != nil {
			return fromStorage(CmdTeamGetStorage, storageErr(marshalErr, "cannot encode team projection"))
		}
		out = append(data, '\n')
		return nil
	})
	return hubCLIResult(req, out, err)
}

// handleCLITeamSet sets a member's state in a team.
func handleCLITeamSet(b *Bus, req HubRequest) HubResponse {
	team, _ := req.Args["team"].(string)
	agent, _ := req.Args["agent"].(string)
	raw, _ := req.Args["state"].(string)
	state, verr := parseEnum[MemberState](raw, "member state")
	if verr != nil {
		return hubCLIResult(req, nil, fromValue(CmdTeamSetInvalidState, verr))
	}
	if !strings.HasSuffix(agent, "@"+req.Caller.Instance) {
		return hubCLIResult(req, nil, opErr(CmdTeamSetNoSuchMember, "a node may only update its own member"))
	}
	text, err := setTeamMember(b, team, agent, state)
	return hubCLIResult(req, []byte(text), err)
}

// handleCLITeamDone marks a team as finished.
func handleCLITeamDone(b *Bus, req HubRequest) HubResponse {
	team, _ := req.Args["team"].(string)
	var denied bool
	if serr := b.locked(func(st *State) { t := st.Teams[team]; denied = t != nil && !teamHasInstance(t, req.Caller.Instance) }); serr != nil {
		return hubCLIResult(req, nil, fromStorage(CmdTeamDoneStorage, serr))
	}
	if denied {
		return hubErrorResponse(req.RequestID, opErr(HubCallIdentity, "team does not belong to this instance"))
	}
	return hubCLIResult(req, []byte{}, finishTeam(b, team))
}

// handleCLITeamCreate creates a team on the hub bus.
func handleCLITeamCreate(b *Bus, req HubRequest) HubResponse {
	raw, ok := req.Args["team_json"].(string)
	if !ok {
		return hubCLIResult(req, nil, opErr(CmdTeamCreateMissingFields, "missing team specification"))
	}
	var spec remoteTeamSpec
	decoder := json.NewDecoder(strings.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&spec); err != nil {
		return hubCLIResult(req, nil, opErr(CmdTeamCreateInvalidJSON, "invalid remote team specification: %v", err))
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return hubCLIResult(req, nil, opErr(CmdTeamCreateInvalidJSON, "expected one team specification"))
	}
	team, err := spec.team(req.Caller.Instance)
	if err != nil {
		return hubCLIResult(req, nil, err)
	}
	return hubCLIResult(req, []byte{}, createRemoteTeamRecord(b, team, req.Caller.Instance))
}

// handleCLIInstances lists the registered instances.
func handleCLIInstances(_ *Bus, instances *HubInstanceRegistry, req HubRequest) HubResponse {
	if instances == nil {
		return hubCLIResult(req, nil, opErr(CmdInstancesStorage, "instance registry is not configured"))
	}
	views, err := instances.liveViews()
	if err != nil {
		return hubCLIResult(req, nil, fromStorage(CmdInstancesStorage, err))
	}
	return hubCLIResult(req, []byte(formatInstances(views)), (*CmdInstancesError)(nil))
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

// handleCLIEnqueue sends fixed node commands to the hub queue without local-state fallback.
func handleCLIEnqueue(b *Bus, instances *HubInstanceRegistry, req HubRequest) HubResponse {
	instance, _ := req.Args["instance"].(string)
	kind, _ := req.Args["kind"].(string)
	task, _ := req.Args["task"].(string)
	params := make(map[string]string)
	raw, _ := req.Args["params"].(map[string]any)
	for key, value := range raw {
		text, ok := value.(string)
		if !ok {
			return hubCLIResult(req, nil, opErr(CommandEnqueueInvalidParams, "command parameters must be strings"))
		}
		params[key] = text
	}
	h := &Hub{Bus: b, Instances: instances}
	return hubCLIResult(req, []byte{}, h.EnqueueCommand(instance, CommandKind(kind), task, params))
}
