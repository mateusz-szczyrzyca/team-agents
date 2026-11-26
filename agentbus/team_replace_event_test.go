package main

import "testing"

func TestMemberReplacedEvent(t *testing.T) {
	ev := memberReplacedEvent{From: "M", To: "N"}
	if got := ev.Text(); got != "M was replaced by N; send to N from now on." {
		t.Fatalf("text = %q", got)
	}
	if got := ev.Kind(); got != MessageKind("member_replaced") || !got.Valid() {
		t.Fatalf("kind = %q, valid = %v", got, got.Valid())
	}
}
