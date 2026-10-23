// Who is connecting through `tailscale serve`. For a device that belongs to a user, `tailscale serve` sets the
// Tailscale-User-Login header. Tagged devices have no user and no login, so for them the server asks the local
// tailscaled (`tailscale whois`) which device the forwarded address belongs to, and lets in the devices — by name or
// by tag — given in --devices. Funnel requests (from the Internet) and addresses outside the tailnet never get in.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os/exec"
	"strings"
	"sync"
	"time"
)

// proxyHeaders are the headers a forwarding proxy adds: a request without any of them is a direct local one.
var proxyHeaders = []string{"X-Forwarded-For", "X-Forwarded-Host", "X-Forwarded-Proto", "Forwarded", "Tailscale-Funnel-Request", "X-Real-Ip"}

// checkIdentity decides who may come through a proxy: a login from --users, a device from --devices; nothing from
// Funnel and nothing from outside the tailnet. A direct local request needs no identity.
func (s *webServer) checkIdentity(r *http.Request) *OpError[AccessCause] {
	login := strings.ToLower(strings.TrimSpace(r.Header.Get("Tailscale-User-Login")))
	if login != "" && s.cfg.users[login] {
		return nil
	}
	if r.Header.Get("Tailscale-Funnel-Request") != "" {
		return webErr(AccessProxiedAnonymous, nil, "a Tailscale Funnel request (from the Internet) is refused")
	}
	proxied := login != ""
	for _, ph := range proxyHeaders {
		proxied = proxied || r.Header.Get(ph) != ""
	}
	if !proxied {
		return nil // a direct local request
	}
	if len(s.cfg.devices) == 0 {
		if login != "" {
			return webErr(AccessUnknownUser, nil, "Tailscale user %s is not allowed (AGENT_WEB_USERS in agents.conf)", login)
		}
		return webErr(AccessProxiedAnonymous, nil, "a proxied request without a Tailscale identity (Funnel or another proxy) is refused — "+
			"tagged devices have no user login: allow them by name or tag in AGENT_WEB_DEVICES")
	}
	ip := lastForwardedFor(r)
	if !isTailnetIP(ip) {
		return webErr(AccessProxiedAnonymous, nil, "a proxied request from outside the tailnet (%q) is refused", ip)
	}
	who, werr := s.whois.lookup(s.cfg.tailscale, ip)
	if werr != nil {
		return webErr(AccessWhoisFailed, werr, "cannot tell which tailnet device %s is", ip)
	}
	if allowedDevice(who, s.cfg.devices) {
		return nil
	}
	tags := strings.Join(who.Node.Tags, ", ")
	if tags == "" {
		tags = "no tags"
	}
	return webErr(AccessUnknownDevice, nil, "device %s (%s) is not allowed (AGENT_WEB_DEVICES in agents.conf)", who.Node.ComputedName, tags)
}

// lastForwardedFor returns the last address of X-Forwarded-For — the one the proxy in front of this server added
// (a client can put anything before it) — without a port.
func lastForwardedFor(r *http.Request) string {
	var last string
	for _, v := range r.Header.Values("X-Forwarded-For") {
		for _, part := range strings.Split(v, ",") {
			if p := strings.TrimSpace(part); p != "" {
				last = p
			}
		}
	}
	if h, _, err := net.SplitHostPort(last); err == nil {
		return h
	}
	return strings.Trim(last, "[]")
}

// tailnetRanges are the address ranges Tailscale gives to tailnet devices.
var tailnetRanges = []*net.IPNet{mustCIDR("100.64.0.0/10"), mustCIDR("fd7a:115c:a1e0::/48")}

// mustCIDR parses a constant network (a typo is a programming error).
func mustCIDR(s string) *net.IPNet {
	_, n, err := net.ParseCIDR(s)
	if err != nil {
		panic(err)
	}
	return n
}

// isTailnetIP reports whether an address belongs to a tailnet device.
func isTailnetIP(s string) bool {
	ip := net.ParseIP(s)
	if ip == nil {
		return false
	}
	for _, n := range tailnetRanges {
		if n.Contains(ip) {
			return true
		}
	}
	return false
}

// allowedDevice reports whether a device is in the allow-list: by short name, full name or one of its tags.
func allowedDevice(who *tsWhois, devices map[string]bool) bool {
	if devices[strings.ToLower(who.Node.ComputedName)] || devices[strings.ToLower(strings.TrimSuffix(who.Node.Name, "."))] {
		return true
	}
	for _, t := range who.Node.Tags {
		if devices[strings.ToLower(t)] {
			return true
		}
	}
	return false
}

// tsNode is the part of a tailnet node that the access check needs (from `tailscale whois --json`).
type tsNode struct {
	Name         string   `json:"Name"`         // full name with a trailing dot ("workstation.example.ts.net.")
	ComputedName string   `json:"ComputedName"` // short name ("workstation")
	Tags         []string `json:"Tags"`         // ACL tags ("tag:trusted")
}

// tsUserProfile is the owner of a tailnet node ("tagged-devices" for tagged nodes).
type tsUserProfile struct {
	LoginName string `json:"LoginName"` // the login
}

// tsWhois is the output of `tailscale whois --json <address>` as far as the access check needs it.
type tsWhois struct {
	Node        tsNode        `json:"Node"`        // the device
	UserProfile tsUserProfile `json:"UserProfile"` // its owner
}

// TailscaleCause: why `tailscale whois` gave no answer.
type TailscaleCause int

const (
	TailscaleRun    TailscaleCause = iota // the CLI could not be run or exited with an error
	TailscaleOutput                       // its output is not the expected JSON
)

// String returns the cause name.
func (c TailscaleCause) String() string {
	switch c {
	case TailscaleRun:
		return "Run"
	case TailscaleOutput:
		return "Output"
	}
	return fmt.Sprintf("TailscaleCause(%d)", int(c))
}

// callTailscale runs `tailscale whois --json <address>` and decodes it. Tests replace it; the test binary's
// default fails the test, like callAgent.
var callTailscale = func(cli, ip string) (*tsWhois, *OpError[TailscaleCause]) {
	out, err := exec.Command(cli, "whois", "--json", ip).Output() // #nosec G204 -- the configured tailscale CLI, an address checked by isTailnetIP
	if err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) {
			return nil, webErr(TailscaleRun, err, "tailscale whois %s: %s", ip, strings.TrimSpace(string(ee.Stderr)))
		}
		return nil, webErr(TailscaleRun, err, "cannot run %s", cli)
	}
	return parseWhois(ip, out)
}

// parseWhois decodes the output of `tailscale whois --json`.
func parseWhois(ip string, out []byte) (*tsWhois, *OpError[TailscaleCause]) {
	var w tsWhois
	if err := json.Unmarshal(out, &w); err != nil {
		return nil, webErr(TailscaleOutput, err, "tailscale whois %s: unexpected output", ip)
	}
	if w.Node.ComputedName == "" && w.Node.Name == "" {
		return nil, webErr(TailscaleOutput, nil, "tailscale whois %s: no device in the answer", ip)
	}
	return &w, nil
}

// whoisEntry is a cached answer of `tailscale whois`.
type whoisEntry struct {
	at  time.Time                // when it was asked
	who *tsWhois                 // the device (nil after a failure)
	err *OpError[TailscaleCause] // the failure (nil after an answer)
}

// whoisCache caches `tailscale whois` per address: a page and its live streams make many requests.
type whoisCache struct {
	mu      sync.Mutex            // guards m
	m       map[string]whoisEntry // by address
	ttl     time.Duration         // how long an answer is reused
	failTTL time.Duration         // how long a failure is reused
}

// lookup returns the device behind a tailnet address.
func (c *whoisCache) lookup(cli, ip string) (*tsWhois, *OpError[TailscaleCause]) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if e, ok := c.m[ip]; ok {
		ttl := c.ttl
		if e.err != nil {
			ttl = c.failTTL
		}
		if time.Since(e.at) < ttl {
			return e.who, e.err
		}
	}
	who, err := callTailscale(cli, ip)
	if c.m == nil {
		c.m = map[string]whoisEntry{}
	}
	c.m[ip] = whoisEntry{at: time.Now(), who: who, err: err}
	return who, err
}
