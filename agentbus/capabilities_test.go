package main

import (
	"os"
	"strings"
	"testing"
)

// fakeDirectory is a capability directory with fixed instances, for tests that do not need a registry.
type fakeDirectory []InstanceView

// Instances returns the fixed instances.
func (f fakeDirectory) Instances() ([]InstanceView, *StorageError) {
	return []InstanceView(f), nil
}

// withDirectory sets the instance name and capability directory of a test server.
func withDirectory(instance string, d CapabilityDirectory) func(*Server) {
	return func(s *Server) { s.instance, s.directory = instance, d }
}

// macView is an instance that provides build:go and os:darwin, as a test fixture.
func macView() InstanceView {
	return InstanceView{Name: "mac", OS: "darwin", Arch: "arm64", Provided: []Capability{"build:go", "os:darwin"},
		Free: 1, Total: 2, Agents: []string{"codex-1 (idle)"}}
}

// linuxView is an instance that provides build:c and os:linux, as a test fixture.
func linuxView() InstanceView {
	return InstanceView{Name: "linux", OS: "linux", Arch: "amd64", Provided: []Capability{"build:c", "os:linux"}}
}

func TestFindCapabilityListsOnlyInstancesThatProvideIt(t *testing.T) {
	s := server(newBus(t), "lead", withDirectory("mac", fakeDirectory{macView(), linuxView()}))
	out := chk(t).ok(toolResult(s.findCapability("build:go")))
	if !strings.Contains(out, "mac (darwin/arm64)") || !strings.Contains(out, "1 of 2 slots free") ||
		!strings.Contains(out, "codex-1 (idle)") {
		t.Fatalf("instance missing from discovery:\n%s", out)
	}
	if strings.Contains(out, "linux") {
		t.Fatalf("instance without the capability listed:\n%s", out)
	}
}

func TestFindCapabilityReportsNoMatchWithLiveInstances(t *testing.T) {
	s := server(newBus(t), "lead", withDirectory("mac", fakeDirectory{macView()}))
	out := chk(t).ok(toolResult(s.findCapability("hw:jlink")))
	if !strings.Contains(out, "No live instance offers hw:jlink") || !strings.Contains(out, "live instances: mac") {
		t.Fatalf("no-match answer: %s", out)
	}
	empty := server(newBus(t), "lead", withDirectory("mac", fakeDirectory{}))
	if out := chk(t).ok(toolResult(empty.findCapability("hw:jlink"))); !strings.Contains(out, "live instances: none") {
		t.Fatalf("empty directory answer: %s", out)
	}
}

func TestFindCapabilityRejectsUnknownCapability(t *testing.T) {
	s := server(newBus(t), "lead", withDirectory("mac", fakeDirectory{macView()}))
	failsWith(t, FindCapabilityInvalidCapability)(s.findCapability("build:cobol"))
}

func TestFindCapabilityWithoutDirectoryFails(t *testing.T) {
	s := server(newBus(t), "lead")
	failsWith(t, FindCapabilityNoDirectory)(s.findCapability("build:go"))
}

func TestFindCapabilityStorageFailureIsNotAnEmptyAnswer(t *testing.T) {
	b := newBus(t)
	reg := &HubInstanceRegistry{Bus: b}
	if _, err := reg.Register("mac", "token:mac", instanceRegisterFixture("mac", "s1")); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(b.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(b.path("state.json"), 0o700); err != nil {
		t.Fatal(err)
	}
	s := server(b, "lead", withDirectory("mac", registryDirectory{registry: reg}))
	failsWith(t, FindCapabilityStorage)(s.findCapability("build:go"))
}

func TestFindCapabilityThroughTheToolDispatcher(t *testing.T) {
	s := server(newBus(t), "lead", withDirectory("mac", fakeDirectory{macView()}))
	out, err := s.callTool("find_capability", map[string]any{"capability": "build:go"})
	if err != nil || !strings.Contains(out, "mac (darwin/arm64)") {
		t.Fatalf("dispatch: %q %v", out, err)
	}
	if _, err := s.callTool("find_capability", map[string]any{"capability": []any{"build:go"}}); err == nil {
		t.Fatal("a non-string capability was accepted")
	}
}

func TestBoardAddRejectsUnrecognizedRequirement(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}})
	lead := server(b, "lead", inTeam("T"))
	failsWith(t, BoardAddInvalidCapability)(lead.boardAddWith("ios build", "", "normal", "", []string{"build:cobol"}))
}

func TestBoardAddStoresRequirementsAndListsThem(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.boardAddWith("ios smoke", "run it", "normal", "", []string{"build:go", "os:darwin"})))
	list := chk(t).ok(toolResult(server(b, "m", inTeam("T")).boardList()))
	if !strings.Contains(list, "requires build:go, os:darwin") {
		t.Fatalf("board_list does not show requires:\n%s", list)
	}
	mustWith(t, b, func(st *State) {
		if got := st.Teams["T"].Board["B1"].Requires; len(got) != 2 || got[0] != "build:go" {
			t.Fatalf("requires not stored: %v", got)
		}
	})
}

func TestBoardAddWithoutRequirementsKeepsOldBehaviour(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAdd("plain", "", "normal")))
	mustWith(t, b, func(st *State) {
		if item := st.Teams["T"].Board["B1"]; len(item.Requires) != 0 {
			t.Fatalf("an item without requires stored %v", item.Requires)
		}
	})
}

func TestBoardClaimEnforcesRequirementsOfTheInstance(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAddWith("ios", "", "normal", "", []string{"build:ios"})))
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAddWith("go", "", "normal", "", []string{"build:go"})))
	m := server(b, "m", inTeam("T"), withDirectory("mac", fakeDirectory{macView()}))
	failsWith(t, BoardClaimMissingCapability)(m.boardClaim("B1"))
	chk(t).ok(toolResult(m.boardClaim("B2")))
}

func TestBoardClaimAlreadyTakenIsReportedBeforeCapabilities(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAddWith("go", "", "normal", "", []string{"build:go"})))
	chk(t).ok(toolResult(server(b, "m", inTeam("T"), withDirectory("mac", fakeDirectory{macView()})).boardClaim("B1")))
	other := server(b, "x", inTeam("T"), withDirectory("linux", fakeDirectory{linuxView()}))
	failsWith(t, BoardClaimAlreadyTaken)(other.boardClaim("B1"))
}

func TestBoardClaimWithoutDirectoryFailsClosedOnlyForRequirements(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.boardAddWith("gated", "", "normal", "", []string{"build:go"})))
	chk(t).ok(toolResult(lead.boardAdd("open", "", "normal")))
	m := server(b, "m", inTeam("T"))
	failsWith(t, BoardClaimMissingCapability)(m.boardClaim("B1"))
	chk(t).ok(toolResult(m.boardClaim("B2")))
}

func TestBoardClaimStorageFailureBlocksItemsWithRequirements(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAddWith("gated", "", "normal", "", []string{"build:go"})))
	reg := &HubInstanceRegistry{Bus: b}
	if err := os.Remove(b.path("state.json")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(b.path("state.json"), 0o700); err != nil {
		t.Fatal(err)
	}
	m := server(b, "m", inTeam("T"), withDirectory("mac", registryDirectory{registry: reg}))
	failsWith(t, BoardClaimStorage)(m.boardClaim("B1"))
}

func TestBoardClaimUsesTheLiveRegistry(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	reg := &HubInstanceRegistry{Bus: b}
	if _, err := reg.Register("mac", "token:mac", instanceRegisterFixture("mac", "s1")); err != nil {
		t.Fatal(err)
	}
	lead := server(b, "lead", inTeam("T"))
	chk(t).ok(toolResult(lead.boardAddWith("go", "", "normal", "", []string{"build:go"})))
	chk(t).ok(toolResult(lead.boardAddWith("ios", "", "normal", "", []string{"build:ios"})))
	// The registry read takes the bus lock; the claim must not deadlock on it.
	m := server(b, "m", inTeam("T"), withDirectory("mac", registryDirectory{registry: reg}))
	chk(t).ok(toolResult(m.boardClaim("B1")))
	failsWith(t, BoardClaimMissingCapability)(m.boardClaim("B2"))
}

func TestBoardAddRejectsNonArrayRequires(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}})
	s := server(b, "lead", inTeam("T"))
	args := map[string]any{"title": "x", "details": "", "level": "normal", "requires": "build:go"}
	if _, err := s.callTool("board_add", args); err == nil {
		t.Fatal("a string in place of the requires array was accepted")
	}
	args["requires"] = []any{"build:go", 3.0}
	if _, err := s.callTool("board_add", args); err == nil {
		t.Fatal("a non-string item in requires was accepted")
	}
	args["requires"] = []any{"build:go"}
	if _, err := s.callTool("board_add", args); err != nil {
		t.Fatalf("valid requires rejected: %v", err)
	}
}

func TestLocalDirectoryParsesDeclaredCapabilities(t *testing.T) {
	d, verr := newLocalDirectory("", " build:go, hw:jlink\tbuild:c ")
	if verr != nil {
		t.Fatalf("valid list rejected: %v", verr)
	}
	views, serr := d.Instances()
	if serr != nil || len(views) != 1 || views[0].Name != "local" {
		t.Fatalf("local view: %+v %v", views, serr)
	}
	for _, c := range []Capability{"build:go", "hw:jlink", "build:c"} {
		if !views[0].Provides(c) {
			t.Fatalf("declared %s not provided: %v", c, views[0].Provided)
		}
	}
}

func TestLocalDirectoryRejectsUnrecognizedCapability(t *testing.T) {
	if _, verr := newLocalDirectory("mac", "build:go, build:cobol"); verr == nil {
		t.Fatal("an unknown capability in INSTANCE_CAPABILITIES was accepted")
	}
}

func TestRegistryDirectoryReportsDeclaredAndPlatformCapabilities(t *testing.T) {
	b := newBus(t)
	reg := &HubInstanceRegistry{Bus: b}
	if _, err := reg.Register("mac", "token:mac", instanceRegisterFixture("mac", "s1")); err != nil {
		t.Fatal(err)
	}
	views, serr := registryDirectory{registry: reg}.Instances()
	if serr != nil || len(views) != 1 {
		t.Fatalf("registry view: %+v %v", views, serr)
	}
	if !views[0].Provides("build:go") || !views[0].Provides("os:linux") || views[0].Provides("build:ios") {
		t.Fatalf("wrong capabilities: %v", views[0].Provided)
	}
	if views[0].Total != 2 || views[0].Free != 1 {
		t.Fatalf("slots not reported: %+v", views[0])
	}
}
