package main

import (
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"
)

func TestTeamReplaceStorageFailuresRestoreMessagesAndReceiptState(t *testing.T) {
	for _, fault := range []string{"state.json.tmp", "messages-full"} {
		t.Run(fault, func(t *testing.T) {
			b := newBus(t)
			from := strings.Repeat("old-member-", 300)
			createTeam(t, b, "T", map[string][3]string{from: {"normal", "", "active"}})
			replacementMessage(t, b, "T", from, "unread")
			mustWith(t, b, func(st *State) { st.Helpers["1"] = &Helper{ID: 1, Status: HelperRunning, AttemptRun: "run-1"} })
			before := replacementState(t, b)
			messages, err := b.messages()
			if err != nil {
				t.Fatal(err)
			}
			if fault == "state.json.tmp" {
				if err := os.Mkdir(b.path(fault), 0700); err != nil {
					t.Fatal(err)
				}
			} else {
				if err := os.WriteFile(b.path(fault), []byte("blocked"), 0600); err != nil {
					t.Fatal(err)
				}
			}
			failsWith(t, CmdTeamReplaceStorage)(teamReplace(b, "T", from, "N"))
			if err := os.Remove(b.path(fault)); err != nil {
				t.Fatal(err)
			}
			after := replacementState(t, b)
			if !reflect.DeepEqual(before, after) {
				t.Fatal("storage failure changed membership or receipt state")
			}
			afterMessages, err := b.messages()
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(messages, afterMessages) {
				t.Fatal("storage failure rewrote messages or published a notice")
			}
		})
	}
}

func TestTeamReplaceRejectsEmptyNamesWithoutMutation(t *testing.T) {
	for _, names := range [][2]string{{"M", ""}, {"", "N"}} {
		b := newBus(t)
		replacementFixture(t, b)
		before := replacementState(t, b)
		_, err := teamReplace(b, "T", names[0], names[1])
		if err == nil || !errors.Is(err, ErrInvalidArgument) {
			t.Fatalf("empty name error = %v, want invalid argument", err)
		}
		if after := replacementState(t, b); !reflect.DeepEqual(before, after) {
			t.Fatal("empty name changed state")
		}
	}
}

func TestTeamReplaceNewLeadReadsNoticeAndPreservesSleepingState(t *testing.T) {
	b := newBus(t)
	createTeam(t, b, "T", map[string][3]string{"L": {"normal", "", "sleeping"}})
	mustWith(t, b, func(st *State) { st.Teams["T"].Lead = "L" })
	wakes := captureWakes(t)
	if _, err := teamReplace(b, "T", "L", "N"); err != nil {
		t.Fatal(err)
	}
	if got := replacementState(t, b).Teams["T"].Members["N"].State; got != MemberSleeping {
		t.Fatalf("state = %s", got)
	}
	if len(*wakes) != 0 {
		t.Fatalf("unexpected wakes: %v", *wakes)
	}
	got := chk(t).ok(toolResult(server(b, "N", inTeam("T")).read(0)))
	if !strings.Contains(got, "L was replaced by N; send to N from now on.") {
		t.Fatalf("new lead read = %q", got)
	}
}
