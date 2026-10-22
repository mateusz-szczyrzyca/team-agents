package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestParseAndValidateCapabilities(t *testing.T) {
	// Valid capabilities
	valid := []string{
		"os:linux", "os:darwin", "build:ios", "build:android", "test:ios-simulator",
		"hw:apple-silicon", "model:claude-3-7-sonnet", "sign:apple-development", "publish:npm",
	}
	for _, c := range valid {
		if !IsRecognizedCapability(c) {
			t.Errorf("expected capability %q to be recognized", c)
		}
	}
	if err := ValidateCapabilities(valid); err != nil {
		t.Fatalf("expected valid capabilities to pass, got %v", err)
	}

	// Sensitive capability check
	if !IsSensitiveCapability("sign:apple-development") {
		t.Errorf("expected sign:apple-development to be sensitive")
	}
	if !IsSensitiveCapability("publish:pypi") {
		t.Errorf("expected publish:pypi to be sensitive")
	}
	if IsSensitiveCapability("build:ios") {
		t.Errorf("expected build:ios not to be sensitive")
	}

	// Invalid capabilities
	invalid := []string{"", "invalid", "bad_ns:foo", ":foo", "build:"}
	for _, c := range invalid {
		if _, _, err := ParseCapability(c); err == nil {
			t.Errorf("expected ParseCapability(%q) to fail", c)
		}
	}
	if err := ValidateCapabilities([]string{"os:linux", "badns:foo"}); err == nil {
		t.Errorf("expected ValidateCapabilities with bad namespace to fail")
	}
}

func sampleManifest(name string) InstanceManifest {
	return InstanceManifest{
		InstanceName: name,
		Timestamp:    Now(),
		TTLSeconds:   180,
		Detected: DetectedCapabilities{
			OS:         "darwin",
			Arch:       "arm64",
			Toolchains: []string{"go1.23", "rust1.82"},
		},
		Declared: []string{"build:ios", "test:ios-simulator"},
		CurrentState: InstanceState{
			Slots: SlotState{Total: 4, Free: 3},
			Agents: []AgentState{
				{Name: "codex-1", Tier: TierNormal, Role: RoleAgent, Status: "idle"},
			},
		},
	}
}

func TestInstanceRegistrationAndHeartbeat(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)

	m := sampleManifest("mac")

	// 1. Success registration
	regResp, regErr := im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	if regErr != nil {
		t.Fatalf("unexpected registration error: %v", regErr)
	}
	if regResp.InstanceName != "mac" || regResp.Lease == 0 || regResp.TTLSeconds != 180 {
		t.Fatalf("unexpected registration response: %+v", regResp)
	}

	// 2. Protocol version mismatch
	_, regErr = im.Register("mac2", InstanceRegisterRequest{
		Version:  99,
		Manifest: sampleManifest("mac2"),
	})
	expectCause(t, regErr, InstanceRegisterProtocolVersion)

	// 3. Instance mismatch
	_, regErr = im.Register("other", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: sampleManifest("mac2"),
	})
	expectCause(t, regErr, InstanceRegisterInstanceMismatch)

	// 4. Invalid declared capability
	badCapManifest := sampleManifest("mac3")
	badCapManifest.Declared = []string{"invalid_no_namespace"}
	_, regErr = im.Register("mac3", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: badCapManifest,
	})
	expectCause(t, regErr, InstanceRegisterInvalidCapability)

	// 5. Active live lease prevents duplicate registration
	_, regErr = im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	expectCause(t, regErr, InstanceRegisterInstanceNameTaken)

	// 6. Successful heartbeat
	m.CurrentState.Slots.Free = 2
	hbResp, hbErr := im.Heartbeat("mac", InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		Sequence:     1,
		Manifest:     m,
	})
	if hbErr != nil {
		t.Fatalf("unexpected heartbeat error: %v", hbErr)
	}
	if hbResp.AcknowledgedSeq != 1 || hbResp.Lease != regResp.Lease {
		t.Fatalf("unexpected heartbeat response: %+v", hbResp)
	}

	// 7. Stale sequence heartbeat
	_, hbErr = im.Heartbeat("mac", InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		Sequence:     1, // already acknowledged 1
		Manifest:     m,
	})
	expectCause(t, hbErr, InstanceHeartbeatStaleHeartbeat)

	// 8. Stale lease heartbeat
	_, hbErr = im.Heartbeat("mac", InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        999, // wrong lease
		Sequence:     2,
		Manifest:     m,
	})
	expectCause(t, hbErr, InstanceHeartbeatStaleLease)

	// 9. Instance not found heartbeat
	_, hbErr = im.Heartbeat("unknown-inst", InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "unknown-inst",
		Lease:        1,
		Sequence:     1,
		Manifest:     sampleManifest("unknown-inst"),
	})
	expectCause(t, hbErr, InstanceHeartbeatInstanceNotFound)

	// 10. Instance mismatch on heartbeat
	_, hbErr = im.Heartbeat("mac", InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "other",
		Lease:        regResp.Lease,
		Sequence:     2,
		Manifest:     sampleManifest("other"),
	})
	expectCause(t, hbErr, InstanceHeartbeatInstanceMismatch)
}

func TestInstanceExpirationAndBoardRelease(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)

	// Register instance "mac"
	m := sampleManifest("mac")
	m.TTLSeconds = 180
	regResp, err := im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	if err != nil {
		t.Fatal(err)
	}

	// Set up team with member "codex-1@mac" claiming board item B1
	mustWith(t, b, func(st *State) {
		tm := &Team{
			ID:   "20261022-test-team",
			Lead: "coordinator",
			Members: map[string]*TeamMember{
				"coordinator": {Agent: "coordinator", State: MemberActive},
				"codex-1@mac": {Agent: "codex-1@mac", State: MemberActive},
			},
			Board: map[string]*BoardItem{
				"B1": {
					ID:       "B1",
					Title:    "Mac Build",
					Level:    LevelNormal,
					Status:   BoardClaimed,
					Assignee: "codex-1@mac",
				},
			},
		}
		st.Teams[tm.ID] = tm
	})

	// Before expiry: ExpireStaleInstances should find 0 expired
	now := time.Now()
	expired, serr := im.ExpireStaleInstances(now)
	if serr != nil {
		t.Fatal(serr)
	}
	if len(expired) != 0 {
		t.Fatalf("expected 0 expired, got %v", expired)
	}

	// Simulate time passing beyond TTL (now + 200s)
	future := now.Add(200 * time.Second)
	expired, serr = im.ExpireStaleInstances(future)
	if serr != nil {
		t.Fatal(serr)
	}
	if len(expired) != 1 || expired[0] != "mac" {
		t.Fatalf("expected ['mac'] expired, got %v", expired)
	}

	// Verify instance status is unreachable
	rec, ok := im.GetInstance("mac")
	if !ok || rec.Status != InstanceUnreachableStatus {
		t.Fatalf("expected unreachable instance, got %+v", rec)
	}

	// Verify member state is unreachable and B1 was released to BoardTodo
	mustWith(t, b, func(st *State) {
		tm := st.Teams["20261022-test-team"]
		member := tm.Members["codex-1@mac"]
		if member.State != MemberUnreachable {
			t.Errorf("expected member state unreachable, got %s", member.State)
		}
		item := tm.Board["B1"]
		if item.Status != BoardTodo || item.Assignee != "" {
			t.Errorf("expected B1 to be released to todo, got status=%s assignee=%s", item.Status, item.Assignee)
		}
	})

	// Re-registration after expiry succeeds and increments lease
	newResp, regErr := im.Register("mac", InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	})
	if regErr != nil {
		t.Fatalf("unexpected registration error after expiry: %v", regErr)
	}
	if newResp.Lease <= regResp.Lease {
		t.Fatalf("expected new lease %d > previous lease %d", newResp.Lease, regResp.Lease)
	}
}

func TestFindCapabilityMCP(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)

	// Register "mac" with build:ios
	m := sampleManifest("mac")
	if _, err := im.Register("mac", InstanceRegisterRequest{Version: hubProtocolVersion, Manifest: m}); err != nil {
		t.Fatal(err)
	}

	// Register "linux" with build:go
	mlinux := sampleManifest("linux")
	mlinux.Detected.OS = "linux"
	mlinux.Declared = []string{"build:go"}
	if _, err := im.Register("linux", InstanceRegisterRequest{Version: hubProtocolVersion, Manifest: mlinux}); err != nil {
		t.Fatal(err)
	}

	s := server(b, "coordinator")

	// Missing capability argument
	_, err := s.findCapability("")
	expectCause(t, err, FindCapabilityMissingCapability)

	// Invalid capability format
	_, err = s.findCapability("invalid_format")
	expectCause(t, err, FindCapabilityInvalidCapability)

	// Search for build:ios
	res, err := s.findCapability("build:ios")
	if err != nil {
		t.Fatalf("unexpected findCapability error: %v", err)
	}
	var matches []CapabilityMatch
	if jsonErr := json.Unmarshal([]byte(res), &matches); jsonErr != nil {
		t.Fatal(jsonErr)
	}
	if len(matches) != 1 || matches[0].Instance != "mac" {
		t.Fatalf("expected match for mac, got %+v", matches)
	}

	// Search for os:linux
	res, err = s.findCapability("os:linux")
	if err != nil {
		t.Fatalf("unexpected findCapability error: %v", err)
	}
	matches = nil
	if jsonErr := json.Unmarshal([]byte(res), &matches); jsonErr != nil {
		t.Fatal(jsonErr)
	}
	if len(matches) != 1 || matches[0].Instance != "linux" {
		t.Fatalf("expected match for linux, got %+v", matches)
	}
}

func TestBoardClaimRequiresCapabilities(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)

	// Register "mac" with build:ios
	m := sampleManifest("mac")
	m.Declared = []string{"build:ios"}
	if _, err := im.Register("mac", InstanceRegisterRequest{Version: hubProtocolVersion, Manifest: m}); err != nil {
		t.Fatal(err)
	}

	// Register "linux" with build:go
	mlinux := sampleManifest("linux")
	mlinux.Declared = []string{"build:go"}
	if _, err := im.Register("linux", InstanceRegisterRequest{Version: hubProtocolVersion, Manifest: mlinux}); err != nil {
		t.Fatal(err)
	}

	// Set up team with board item requiring "build:ios"
	mustWith(t, b, func(st *State) {
		tm := &Team{
			ID:   "20261022-cap-team",
			Lead: "lead",
			Members: map[string]*TeamMember{
				"agent@mac":   {Agent: "agent@mac", State: MemberActive, Tier: TierNormal},
				"agent@linux": {Agent: "agent@linux", State: MemberActive, Tier: TierNormal},
			},
			Board: map[string]*BoardItem{
				"B1": {
					ID:       "B1",
					Title:    "iOS task",
					Details:  "run iOS tests",
					Level:    LevelNormal,
					Status:   BoardTodo,
					Requires: []string{"build:ios"},
				},
			},
		}
		st.Teams[tm.ID] = tm
	})

	// Member on linux lacks build:ios -> BoardClaimMissingCapability
	sLinux := server(b, "agent@linux")
	sLinux.team = "20261022-cap-team"
	_, err := sLinux.boardClaim("B1")
	expectCause(t, err, BoardClaimMissingCapability)

	// Member on mac has build:ios -> succeeds
	sMac := server(b, "agent@mac")
	sMac.team = "20261022-cap-team"
	out, err := sMac.boardClaim("B1")
	if err != nil {
		t.Fatalf("unexpected claim error: %v", err)
	}
	if out == "" {
		t.Fatalf("expected non-empty claim output")
	}
}

func TestHubInstanceHTTPRoutes(t *testing.T) {
	b := newBus(t)
	h := &Hub{
		Bus: b,
		Authenticate: func(r *http.Request) (string, *HubAuthError) {
			token := r.Header.Get("Authorization")
			if token == "Bearer mac-token" {
				return "mac", nil
			}
			return "", opErr(HubAuthWrongToken, "invalid token")
		},
	}

	m := sampleManifest("mac")
	regReq := InstanceRegisterRequest{
		Version:  hubProtocolVersion,
		Manifest: m,
	}
	body, _ := json.Marshal(regReq)

	// 1. Unauthenticated request
	w := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/v1/instances/register", bytes.NewReader(body))
	h.ServeHTTP(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401 Unauthorized, got %d", w.Code)
	}

	// 2. Successful registration
	w = httptest.NewRecorder()
	req = httptest.NewRequest(http.MethodPost, "/v1/instances/register", bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200 OK, got %d: %s", w.Code, w.Body.String())
	}
	var regResp InstanceRegisterResponse
	if err := json.Unmarshal(w.Body.Bytes(), &regResp); err != nil {
		t.Fatal(err)
	}
	if regResp.InstanceName != "mac" || regResp.Lease == 0 {
		t.Fatalf("unexpected response: %+v", regResp)
	}

	// 3. Heartbeat via HTTP
	hbReq := InstanceHeartbeatRequest{
		Version:      hubProtocolVersion,
		InstanceName: "mac",
		Lease:        regResp.Lease,
		Sequence:     1,
		Manifest:     m,
	}
	hbBody, _ := json.Marshal(hbReq)
	w = httptest.NewRecorder()
	req = httptest.NewRequest(http.MethodPost, "/v1/instances/heartbeat", bytes.NewReader(hbBody))
	req.Header.Set("Authorization", "Bearer mac-token")
	h.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200 OK for heartbeat, got %d: %s", w.Code, w.Body.String())
	}
}

func TestValidateLease(t *testing.T) {
	b := newBus(t)
	im := NewInstanceManager(b)

	// 1. Not found
	st, rec := im.ValidateLease("unknown", 1)
	if st != LeaseNotFound || rec != nil {
		t.Fatalf("expected LeaseNotFound, got %v", st)
	}

	// Register instance
	m := sampleManifest("mac")
	m.CurrentState.Agents[0].Repo = "team-agents"
	m.CurrentState.Agents[0].Branch = "main"
	m.CurrentState.Agents[0].Worktree = "/tmp/worktree"
	regResp, err := im.Register("mac", InstanceRegisterRequest{Version: hubProtocolVersion, Manifest: m})
	if err != nil {
		t.Fatal(err)
	}

	// 2. Valid lease
	st, rec = im.ValidateLease("mac", regResp.Lease)
	if st != LeaseValid || rec == nil || rec.Manifest.CurrentState.Agents[0].Repo != "team-agents" {
		t.Fatalf("expected LeaseValid, got %v, rec=%+v", st, rec)
	}

	// 3. Stale lease
	st, rec = im.ValidateLease("mac", regResp.Lease+1)
	if st != LeaseStale || rec == nil {
		t.Fatalf("expected LeaseStale, got %v", st)
	}
}
