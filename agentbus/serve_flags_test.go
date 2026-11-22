package main

// Tests of the flags of serve (written first): the server of a run is made from them, so a run of a role that keeps a
// plan reaches its plan and the agent command that checks its requests against it.

import (
	"encoding/json"
	"io"
	"testing"
)

func TestServeGivesItsServerThePlanAndTheAgentCommand(t *testing.T) {
	fs := newFlagSet("serve")
	flags := defineServeFlags(fs)
	err := fs.Parse([]string{"--agent", "m", "--team", "T", "--tier", "normal", "--role", "investigator",
		"--plan", "/w/.agent/plan.md", "--agent-cmd", "agent", "--home", "/h", "--dir", "/h/bus"})
	if err != nil {
		t.Fatal(err)
	}

	s := flags.server(RoleInvestigator, TierNormal, json.NewEncoder(io.Discard))

	if s.plan != "/w/.agent/plan.md" || s.agentCmd != "agent" || s.home != "/h" {
		t.Fatalf("the server should have the plan, the agent command and the home: plan %q, agent %q, home %q",
			s.plan, s.agentCmd, s.home)
	}
	if s.me != "m" || s.team != "T" || s.role != RoleInvestigator || s.tier != TierNormal || s.bus.dir != "/h/bus" {
		t.Fatalf("the server should be the run's: %+v", s)
	}
	if s.tasksDir != "/h/tasks" {
		t.Fatalf("without --tasks-dir the tasks are beside the bus, got %q", s.tasksDir)
	}
}

func TestServeWithoutAPlanGivesItsServerNone(t *testing.T) {
	fs := newFlagSet("serve")
	flags := defineServeFlags(fs)
	if err := fs.Parse([]string{"--agent", "w", "--tier", "worker"}); err != nil {
		t.Fatal(err)
	}

	if s := flags.server(RoleAgent, TierWorker, json.NewEncoder(io.Discard)); s.plan != "" || s.agentCmd != "" {
		t.Fatalf("a run without a plan has none: plan %q, agent %q", s.plan, s.agentCmd)
	}
}
