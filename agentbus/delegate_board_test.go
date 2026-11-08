package main

import "testing"

// TestDelegateRefusesBoardInstructions: a worker has no board tools — no board_claim, board_done or board_list — and
// sees nothing of the team's board. A request telling it to claim or finish a board item, or to take "the newest
// easy item", cannot be done (H979, H983); it is refused at once, with what to write instead.
func TestDelegateRefusesBoardInstructions(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	for _, req := range []string{
		"Claim and perform easy board item R10B coordinator read-only source inventory (newest easy item)",
		"Perform the newest easy board item and report back",
		"Use board_claim on B47, then board_done with the result",
		"Take the next easy item and do it",
	} {
		failsWith(t, DelegateBoardInstruction)(n.delegate(req, ""))
	}
}

// TestDelegateAcceptsACopiedBoardItem: a request that names a board item but carries its content — what the script
// writes for an easy item nobody owns — is a request a worker can do.
func TestDelegateAcceptsACopiedBoardItem(t *testing.T) {
	b := newBus(t)
	n := server(b, "n1")
	chk(t).ok(toolResult(n.delegate("count the Go files — board item B47 of team T: run `find /repo/agent -name '*.go' | wc -l` and return the number", "")))
}
