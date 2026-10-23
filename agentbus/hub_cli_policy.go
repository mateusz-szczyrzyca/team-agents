// Instance operator CLI admission never accepts executable paths or agent run authority.
package main

import (
	"encoding/json"
	"fmt"
	"strings"
)

// hubOperator recognizes only the control-plane envelope, separately from registered agent runs.
func hubOperator(c HubCaller) bool {
	return c.Agent == "cli" && c.Task == "cli" && c.Team == "" && c.Tier == TierNormal && c.Role == RoleAgent && validInstanceName(c.Instance)
}

// hubCLIResult preserves operation causes and exact stdout at the CLI dispatcher boundary.
func hubCLIResult[C opCause](req HubRequest, out []byte, err *OpError[C]) HubResponse {
	if err != nil {
		return HubResponse{RequestID: req.RequestID, Error: hubFailure(err)}
	}
	return HubResponse{RequestID: req.RequestID, Stdout: &out}
}

// remoteTeamSpec contains shared assignments only; local executable and home paths have no wire representation.
type remoteTeamSpec struct {
	ID      string                 `json:"id"`                     // validated team identifier
	Repo    string                 `json:"repo"`                   // repository identifier
	Lead    string                 `json:"lead"`                   // qualified coordinator, empty before election
	Members map[string]*TeamMember `json:"members"`                // qualified, independently validated run assignments
	Resumed string                 `json:"resumed_from,omitempty"` // previous team identifier
}

// remoteTeamJSON drops local process configuration before forwarding an existing CLI team input.
func remoteTeamJSON(t *Team) (string, *CmdTeamCreateError) {
	data, err := json.Marshal(remoteTeamSpec{ID: t.ID, Repo: t.Repo, Lead: t.Lead, Members: t.Members, Resumed: t.Resumed})
	if err != nil {
		return "", fromStorage(CmdTeamCreateStorage, storageErr(err, "cannot encode remote team specification"))
	}
	return string(data), nil
}

// team validates qualified ownership before any data can reach the hub's local wake fallback.
func (s remoteTeamSpec) team(instance string) (*Team, *CmdTeamCreateError) {
	if !validNodeTaskID(s.ID) || !validNodeTaskID(s.Repo) || len(s.Members) == 0 || (s.Resumed != "" && !validNodeTaskID(s.Resumed)) {
		return nil, opErr(CmdTeamCreateMissingFields, "remote team needs valid id, repository and members")
	}
	t := &Team{ID: s.ID, Repo: s.Repo, Lead: s.Lead, Members: s.Members, Resumed: s.Resumed}
	for name, m := range t.Members {
		base, owner, found := strings.Cut(name, "@")
		if m == nil || !found || !validNodeTaskID(base) || !validInstanceName(owner) || m.Agent != name || !validNodeTaskID(m.Task) || !m.Tier.Valid() || !m.State.Valid() {
			return nil, opErr(CmdTeamCreateMissingFields, "member %q needs a qualified identity, valid task, tier and state", name)
		}
	}
	if !teamHasInstance(t, instance) || (t.Lead != "" && t.Members[t.Lead] == nil) {
		return nil, opErr(CmdTeamCreateMissingFields, "team must include this instance and its named coordinator")
	}
	return t, nil
}

// teamHasInstance limits operator reads and lifecycle changes to teams containing their own node.
func teamHasInstance(t *Team, instance string) bool {
	for name := range t.Members {
		if strings.HasSuffix(name, "@"+instance) {
			return true
		}
	}
	return false
}

// formatInstances is shared by local and forwarded CLI output and never serializes private lease credentials.
func formatInstances(views []InstanceView) string {
	var out strings.Builder
	for _, v := range views {
		caps := make([]string, len(v.Provided))
		for i, c := range v.Provided {
			caps[i] = string(c)
		}
		fmt.Fprintf(&out, "%s\t%s/%s\t%d/%d\t%s\n", v.Name, v.OS, v.Arch, v.Free, v.Total, strings.Join(caps, ","))
	}
	return out.String()
}

// createRemoteTeamRecord fences collisions and copies resume state on the hub without exposing private quiz data.
func createRemoteTeamRecord(b *Bus, team *Team, instance string) *CmdTeamCreateError {
	team.Created = Now()
	return transact(b, CmdTeamCreateStorage, func(st *State) *CmdTeamCreateError {
		if st.Teams[team.ID] != nil {
			return opErr(CmdTeamCreateAlreadyExists, "team %s already exists", team.ID)
		}
		if team.Resumed != "" {
			previous := st.Teams[team.Resumed]
			if previous == nil || previous.Repo != team.Repo || !teamHasInstance(previous, instance) {
				return opErr(CmdTeamCreateMissingFields, "previous team is unavailable to this instance")
			}
			// A JSON clone avoids aliasing old and new team boards and private quiz state.
			data, err := json.Marshal(previous)
			if err != nil {
				return fromStorage(CmdTeamCreateStorage, storageErr(err, "cannot encode previous team"))
			}
			var clone Team
			if err := json.Unmarshal(data, &clone); err != nil {
				return fromStorage(CmdTeamCreateStorage, storageErr(err, "cannot clone previous team"))
			}
			team.Quiz, team.Board, team.NextBID, team.BoardRev = clone.Quiz, clone.Board, clone.NextBID, clone.BoardRev
			team.Consults, team.NextCID = clone.Consults, clone.NextCID
		}
		return storeTeamRecord(b, st, team)
	})
}
