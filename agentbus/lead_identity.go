package main

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os/exec"
	"strings"
	"time"
)

// leadIdentity is the authenticated peer identity of a lead HTTP request.
type leadIdentity struct {
	Node  string // the authenticated short node name
	Local bool   // whether the request originated from the local machine (127.0.0.1 or ::1)
}

// leadWhoisNode is the device information in the `tailscale whois --json` output.
type leadWhoisNode struct {
	Name         string   `json:"Name"`         // full DNS name ("node.example.ts.net.")
	ComputedName string   `json:"ComputedName"` // short device name ("node")
	Tags         []string `json:"Tags"`         // ACL tags ("tag:runners")
}

// leadWhoisResponse is the output schema of `tailscale whois --json <address>`.
type leadWhoisResponse struct {
	Node leadWhoisNode `json:"Node"` // the Tailscale device
}

// parseLeadWhois decodes the JSON output of `tailscale whois --json <address>`.
func parseLeadWhois(ip string, out []byte) (*leadWhoisResponse, *LeadWhoisError) {
	var resp leadWhoisResponse
	if err := json.Unmarshal(out, &resp); err != nil {
		return nil, opErr(LeadWhoisMalformedOutput, "tailscale whois %s: unexpected output: %v", ip, err)
	}
	if resp.Node.ComputedName == "" && resp.Node.Name == "" {
		return nil, opErr(LeadWhoisMalformedOutput, "tailscale whois %s: no device in the answer", ip)
	}
	return &resp, nil
}

// callTailscaleWhois executes `tailscale whois --json <ip>` or a test fake.
var callTailscaleWhois = func(cli, ip string) (*leadWhoisResponse, *LeadWhoisError) {
	if cli == "" {
		cli = "tailscale"
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, cli, "whois", "--json", ip).Output() // #nosec G204 -- fixed whois subcommand with validated IP and bounded timeout
	if err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) {
			return nil, opErr(LeadWhoisTailscaleFailed, "tailscale whois %s: %s", ip, strings.TrimSpace(string(ee.Stderr)))
		}
		return nil, opErr(LeadWhoisTailscaleFailed, "cannot run %s: %v", cli, err)
	}
	return parseLeadWhois(ip, out)
}

// isLeadNodeAllowed reports whether a device matches the lead allowlist by name or tag.
func isLeadNodeAllowed(who *leadWhoisResponse, allowed []string) bool {
	if len(allowed) == 0 {
		return false
	}
	short := who.Node.ComputedName
	if short == "" {
		short = strings.TrimSuffix(who.Node.Name, ".")
		if idx := strings.IndexByte(short, '.'); idx != -1 {
			short = short[:idx]
		}
	}
	short = strings.ToLower(short)
	full := strings.ToLower(strings.TrimSuffix(who.Node.Name, "."))
	for _, a := range allowed {
		item := strings.ToLower(strings.TrimSpace(a))
		if item == "" {
			continue
		}
		if item == short || item == full {
			return true
		}
		for _, tag := range who.Node.Tags {
			if item == strings.ToLower(tag) {
				return true
			}
		}
	}
	return false
}

// identifyLeadPeer identifies and authorizes an incoming HTTP peer by its socket RemoteAddr.
// Exact 127.0.0.1 and ::1 are authorized as local lead requests without consulting whois.
// Other loopback addresses (including 127.0.0.2) and tailnet addresses are verified through Tailscale whois.
func identifyLeadPeer(remote string, localNode string, allowed []string) (leadIdentity, *LeadWhoisError) {
	host := remote
	if h, _, err := net.SplitHostPort(remote); err == nil {
		host = h
	}
	host = strings.Trim(host, "[]")

	if host == "127.0.0.1" || host == "::1" {
		return leadIdentity{Node: localNode, Local: true}, nil
	}

	ip := net.ParseIP(host)
	if ip == nil {
		return leadIdentity{}, opErr(LeadWhoisUntrustedNode, "invalid remote address %q", remote)
	}

	who, werr := callTailscaleWhois("", host)
	if werr != nil {
		return leadIdentity{}, werr
	}

	shortName := who.Node.ComputedName
	if shortName == "" {
		shortName = strings.TrimSuffix(who.Node.Name, ".")
		if idx := strings.IndexByte(shortName, '.'); idx != -1 {
			shortName = shortName[:idx]
		}
	}

	if !isLeadNodeAllowed(who, allowed) {
		tags := strings.Join(who.Node.Tags, ", ")
		if tags == "" {
			tags = "no tags"
		}
		return leadIdentity{}, opErr(LeadWhoisUntrustedNode, "device %s (%s) is not allowed by lead configuration", shortName, tags)
	}

	return leadIdentity{Node: shortName, Local: false}, nil
}
