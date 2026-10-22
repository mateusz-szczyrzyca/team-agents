package main

import (
	"context"
	"testing"
	"time"
)

// TestLeadCallWaitsThroughPendingPolls checks that the lead's in-process call keeps polling a pending
// operation until its final response arrives, without HTTP.
func TestLeadCallWaitsThroughPendingPolls(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	response := leadCall(context.Background(), h, "mac", req)
	if response.Pending != nil {
		t.Fatalf("leadCall returned Pending without a cancelled context: %+v", response)
	}
	if response.Error != nil || response.Result == nil {
		t.Fatalf("leadCall did not return the operation result: %+v", response)
	}
}

// TestLeadCallRejectsOtherInstanceName checks that the in-process path keeps the same identity rule as
// HTTP: a caller that does not belong to the instance named by the lead is refused.
func TestLeadCallRejectsOtherInstanceName(t *testing.T) {
	h, req := testHub(t)
	h.PollDuration = time.Millisecond
	response := leadCall(context.Background(), h, "pi5", req)
	if response.Error == nil || response.Error.Cause != HubCallIdentity.String() {
		t.Fatalf("got %+v, want an Identity error", response)
	}
}
