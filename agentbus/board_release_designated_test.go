package main

import "testing"

// TestReleaseItemsOfFreesDesignatedItems: an item designated for a member who is out (failed, paused long, or done)
// and not yet claimed goes back to the board too — nobody else could take it while it is reserved for that member.
func TestReleaseItemsOfFreesDesignatedItems(t *testing.T) {
	team := &Team{Board: map[string]*BoardItem{
		"B1": {ID: "B1", Status: BoardTodo, For: "m", Level: LevelNormal},
		"B2": {ID: "B2", Status: BoardTodo, For: "other", Level: LevelNormal},
		"B3": {ID: "B3", Status: BoardDone, For: "m", Assignee: "m", Level: LevelNormal},
	}}
	got := team.releaseItemsOf("m")
	if len(got) != 1 || got[0] != "B1" {
		t.Fatalf("released %v, want [B1]", got)
	}
	if it := team.Board["B1"]; it.For != "" || it.Previous != "m" || it.Status != BoardTodo {
		t.Fatalf("B1 after release: for=%q previous=%q status=%q", it.For, it.Previous, it.Status)
	}
	if team.Board["B2"].For != "other" || team.Board["B3"].Status != BoardDone {
		t.Fatalf("another member's item or a done one changed: %+v %+v", *team.Board["B2"], *team.Board["B3"])
	}
}
