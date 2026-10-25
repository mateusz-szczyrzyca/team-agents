package main

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

// callTailscaleWhois executes `tailscale whois --json <ip>` or a test fake.
var callTailscaleWhois = func(_, _ string) (*leadWhoisResponse, *LeadWhoisError) {
	return nil, nil
}

// identifyLeadPeer identifies and authorizes an incoming HTTP peer by its socket RemoteAddr.
// Exact 127.0.0.1 and ::1 are authorized as local lead requests without consulting whois.
// Other loopback addresses (including 127.0.0.2) and tailnet addresses are verified through Tailscale whois.
func identifyLeadPeer(_, _ string, _ []string) (leadIdentity, *LeadWhoisError) {
	return leadIdentity{}, nil
}
