package main

import (
	"os"
	"path/filepath"
	"testing"
)

// signingView is an instance that provides a sensitive capability, as a test fixture.
func signingView() InstanceView {
	return InstanceView{Name: "mac", OS: "darwin", Arch: "arm64", Provided: []Capability{"build:ios", "sign:apple"}}
}

// writeApprovals stores the approvals file under home, creating node/ like the node executor expects.
func writeApprovals(t *testing.T, home, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(home, "node"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(home, approvedCapabilitiesFile), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
}

// signingBoard creates a team with an item that requires a sensitive capability and returns a member on instance mac
// with the given home.
func signingBoard(t *testing.T, home string) (*Bus, *Server) {
	t.Helper()
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAddWith("sign", "", "normal", "", []string{"sign:apple"})))
	m := server(b, "m", inTeam("T"), withDirectory("mac", fakeDirectory{signingView()}), func(s *Server) { s.home = home })
	return b, m
}

func TestBoardClaimSensitiveNeedsOperatorApproval(t *testing.T) {
	home := t.TempDir()
	b, m := signingBoard(t, home)
	failsWith(t, BoardClaimNeedsApproval)(m.boardClaim("B1"))
	mustWith(t, b, func(st *State) {
		if status := st.Teams["T"].Board["B1"].Status; status != BoardTodo {
			t.Fatalf("a refused claim changed the item to %s", status)
		}
	})
}

func TestBoardClaimSensitiveWithApprovalSucceeds(t *testing.T) {
	home := t.TempDir()
	writeApprovals(t, home, `["sign:apple"]`)
	_, m := signingBoard(t, home)
	chk(t).ok(toolResult(m.boardClaim("B1")))
}

func TestBoardClaimApprovalIsPerCapability(t *testing.T) {
	home := t.TempDir()
	writeApprovals(t, home, `["publish:npm"]`)
	_, m := signingBoard(t, home)
	failsWith(t, BoardClaimNeedsApproval)(m.boardClaim("B1"))
}

func TestBoardClaimSensitiveFailsClosedWithoutLocalHome(t *testing.T) {
	// Servers run by the hub have no local AGENTS_HOME; nothing can be approved for them.
	_, m := signingBoard(t, "")
	failsWith(t, BoardClaimNeedsApproval)(m.boardClaim("B1"))
}

func TestBoardClaimMalformedApprovalsIsStorageFailure(t *testing.T) {
	home := t.TempDir()
	writeApprovals(t, home, `["not:a:capability"]`)
	_, m := signingBoard(t, home)
	failsWith(t, BoardClaimStorage)(m.boardClaim("B1"))
}

func TestBoardClaimOrdinaryItemIgnoresBrokenApprovals(t *testing.T) {
	// A broken approvals file must not block items without a sensitive requirement.
	home := t.TempDir()
	writeApprovals(t, home, `{broken`)
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	chk(t).ok(toolResult(server(b, "lead", inTeam("T")).boardAdd("plain", "", "normal")))
	m := server(b, "m", inTeam("T"), withDirectory("mac", fakeDirectory{signingView()}), func(s *Server) { s.home = home })
	chk(t).ok(toolResult(m.boardClaim("B1")))
}

func TestBoardClaimMissingApprovalsFileApprovesNothing(t *testing.T) {
	_, m := signingBoard(t, t.TempDir())
	failsWith(t, BoardClaimNeedsApproval)(m.boardClaim("B1"))
}
