// What a team waits for. A member asleep while a worker (delegate) or an investigator works on its request is not
// stuck: the result wakes it. The stuck detection of the agent script asks `agentbus team-waiting` before it calls a
// team stuck (bin/agent: team_after_run).
package main

import (
	"fmt"
	"sort"
)

// open reports whether a worker request has not finished yet (its result is still to come).
func (s HelperStatus) open() bool {
	switch s {
	case HelperQueued, HelperStarting, HelperRunning:
		return true
	}
	return false
}

// open reports whether an investigation has not finished yet (its result is still to come).
func (s InvestigationStatus) open() bool {
	switch s {
	case InvQueued, InvWaitroom, InvStarting, InvRunning, InvWaitingHuman:
		return true
	}
	return false
}

// teamWaitingOn returns what the members of a team wait for — their unfinished worker requests and investigations —
// as "H12 (worker request of lead, running)" and "#7 (investigation of member, queued)", sorted. Nil: nothing.
func teamWaitingOn(st *State, team string) []string {
	t := st.Teams[team]
	if t == nil {
		return nil
	}
	tasks := map[string]bool{}
	for _, m := range t.Members {
		if m.Task != "" {
			tasks[m.Task] = true
		}
	}
	var out []string
	for _, h := range st.Helpers {
		if tasks[h.FromTask] && h.Status.open() {
			out = append(out, fmt.Sprintf("H%d (worker request of %s, %s)", h.ID, h.From, h.Status))
		}
	}
	for _, inv := range st.Investigations {
		if tasks[inv.FromTask] && inv.Status.open() {
			out = append(out, fmt.Sprintf("#%d (investigation of %s, %s)", inv.ID, inv.FromAgent, inv.Status))
		}
	}
	sort.Strings(out)
	return out
}

// cmdTeamWaiting — team-waiting: what a team's members wait for, as JSON ({"waiting": [...]}; empty: nothing).
//
//	agentbus team-waiting --dir DIR --team ID
func cmdTeamWaiting(args []string) {
	fs := newFlagSet("team-waiting")
	dir := fs.String("dir", defaultDir(), "state directory")
	team := fs.String("team", "", "team ID")
	parseFlags(fs, args)
	b := &Bus{dir: *dir}
	exitOn(transact(b, CmdTeamWaitingStorage, func(st *State) *CmdTeamWaitingError {
		if st.Teams[*team] == nil {
			return opErr(CmdTeamWaitingNoSuchTeam, "no team %s", *team)
		}
		waiting := teamWaitingOn(st, *team)
		if waiting == nil {
			waiting = []string{}
		}
		return fromStorage(CmdTeamWaitingStorage, printJSON(map[string][]string{"waiting": waiting}))
	}).asError())
}
