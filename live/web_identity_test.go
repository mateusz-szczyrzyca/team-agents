package main

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// whoisFixture has the shape of a real `tailscale whois --json` answer for a tagged device (identifiers, keys and
// addresses are made up).
const whoisFixture = `{
  "Node": {
    "ID": 1111111111111111,
    "StableID": "nExampleStable1CNTRL",
    "Name": "workstation.example-tailnet.ts.net.",
    "User": 2222222222222222,
    "Key": "nodekey:0000000000000000000000000000000000000000000000000000000000000000",
    "Addresses": ["100.64.0.5/32", "fd7a:115c:a1e0::5/128"],
    "Hostinfo": {"OS": "macOS", "Hostname": "workstation"},
    "Tags": ["tag:importantm", "tag:trusted"],
    "Online": true,
    "ComputedName": "workstation",
    "ComputedNameWithHost": "workstation"
  },
  "UserProfile": {"ID": 2222222222222222, "LoginName": "tagged-devices", "DisplayName": "Tagged Devices"},
  "CapMap": null
}`

// stubWhois answers `tailscale whois` from the fixture for 100.64.0.5 and fails for anything else; it counts calls.
func stubWhois(t *testing.T) *int {
	t.Helper()
	calls := 0
	prev := callTailscale
	callTailscale = func(_ string, ip string) (*tsWhois, *OpError[TailscaleCause]) {
		calls++
		if ip == "100.64.0.5" {
			return parseWhois(ip, []byte(whoisFixture))
		}
		return nil, webErr(TailscaleRun, errors.New("exit status 1"), "tailscale whois %s: no match for IP", ip)
	}
	t.Cleanup(func() { callTailscale = prev })
	return &calls
}

// deviceServer is a web server that lets in the given devices.
func deviceServer(t *testing.T, devices string) *webServer {
	t.Helper()
	home := fixture(t)
	cfg := webConfig{config: config{home: home, busDir: filepath.Join(home, "bus"), agentCmd: "/nonexistent/agent", queueMax: 3},
		users: map[string]bool{}, devices: parseUsers(devices), tailscale: "tailscale", poll: 30 * time.Millisecond}
	s, serr := newWebServer(cfg)
	if serr != nil {
		t.Fatal(serr)
	}
	return s
}

// servedReq is a GET as `tailscale serve` forwards it: from loopback, with X-Forwarded-For (and maybe more headers).
func servedReq(hdr ...string) *http.Request {
	r := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:8787/", nil)
	r.RemoteAddr = "127.0.0.1:40000"
	r.Header.Set("X-Forwarded-Proto", "https")
	for i := 0; i+1 < len(hdr); i += 2 {
		r.Header.Add(hdr[i], hdr[i+1])
	}
	return r
}

// causeOf returns the access cause of a refusal (-1 when the request was let in).
func causeOf(t *testing.T, e *OpError[AccessCause]) AccessCause {
	t.Helper()
	if e == nil {
		return -1
	}
	var oe *OpError[AccessCause]
	if err := error(e); !errors.As(err, &oe) || !errors.Is(err, ErrWeb) {
		t.Fatalf("not a typed access error: %v", e)
	}
	return oe.Cause
}

func TestTaggedDevicesAreLetInByNameOrTag(t *testing.T) {
	stubWhois(t)
	for _, devices := range []string{"workstation", "WORKSTATION", "workstation.example-tailnet.ts.net", "tag:trusted", "iphone, tag:importantm"} {
		s := deviceServer(t, devices)
		if c := causeOf(t, s.checkAccess(servedReq("X-Forwarded-For", "100.64.0.5"))); c != -1 {
			t.Errorf("devices %q: refused (%v)", devices, c)
		}
	}
}

func TestDeviceRefusals(t *testing.T) {
	calls := stubWhois(t)
	s := deviceServer(t, "iphone tag:admin")
	cases := []struct {
		name string
		r    *http.Request
		want AccessCause
		msg  string
	}{
		{"a device not on the list", servedReq("X-Forwarded-For", "100.64.0.5"), AccessUnknownDevice, "device workstation (tag:importantm, tag:trusted) is not allowed"},
		{"whois does not know the address", servedReq("X-Forwarded-For", "100.64.0.9"), AccessWhoisFailed, "cannot tell which tailnet device 100.64.0.9 is"},
		{"an address outside the tailnet (Funnel-like)", servedReq("X-Forwarded-For", "203.0.113.9"), AccessProxiedAnonymous, "outside the tailnet"},
		{"a forged first address does not count", servedReq("X-Forwarded-For", "100.64.0.5, 203.0.113.9"), AccessProxiedAnonymous, "outside the tailnet"},
		{"Funnel", servedReq("X-Forwarded-For", "100.64.0.5", "Tailscale-Funnel-Request", "?1"), AccessProxiedAnonymous, "Funnel"},
	}
	for _, c := range cases {
		e := s.checkAccess(c.r)
		if got := causeOf(t, e); got != c.want || !strings.Contains(e.Message, c.msg) {
			t.Errorf("%s: %v %v, want %v containing %q", c.name, got, e, c.want, c.msg)
		}
	}
	if *calls != 2 {
		t.Errorf("tailscale whois called %d times, want 2 (never for addresses outside the tailnet or for Funnel)", *calls)
	}
}

func TestLastForwardedForIsTheProxysAddress(t *testing.T) {
	for _, c := range []struct {
		hdr  []string
		want string
	}{
		{[]string{"100.64.0.5"}, "100.64.0.5"},
		{[]string{"1.2.3.4, 100.64.0.5"}, "100.64.0.5"},
		{[]string{"1.2.3.4", "100.64.0.5:41641"}, "100.64.0.5"},
		{[]string{"[fd7a:115c:a1e0::5]:443"}, "fd7a:115c:a1e0::5"},
	} {
		r := servedReq()
		for _, h := range c.hdr {
			r.Header.Add("X-Forwarded-For", h)
		}
		if got := lastForwardedFor(r); got != c.want {
			t.Errorf("%v: %q, want %q", c.hdr, got, c.want)
		}
	}
	for ip, want := range map[string]bool{"100.64.0.5": true, "100.127.255.1": true, "100.128.0.1": false, "fd7a:115c:a1e0::5": true, "10.0.0.1": false, "": false} {
		if isTailnetIP(ip) != want {
			t.Errorf("isTailnetIP(%q) != %v", ip, want)
		}
	}
}

func TestWhoisAnswersAreCached(t *testing.T) {
	calls := stubWhois(t)
	s := deviceServer(t, "workstation")
	for i := 0; i < 5; i++ {
		if c := causeOf(t, s.checkAccess(servedReq("X-Forwarded-For", "100.64.0.5"))); c != -1 {
			t.Fatalf("refused: %v", c)
		}
	}
	if *calls != 1 {
		t.Errorf("tailscale whois called %d times for one device, want 1", *calls)
	}
}

func TestWhoisOutputErrors(t *testing.T) {
	for out, want := range map[string]TailscaleCause{"not json": TailscaleOutput, `{"Node": {}}`: TailscaleOutput} {
		_, e := parseWhois("100.64.0.5", []byte(out))
		var oe *OpError[TailscaleCause]
		if err := error(e); e == nil || !errors.As(err, &oe) || oe.Cause != want {
			t.Errorf("%q: %v", out, e)
		}
	}
}
