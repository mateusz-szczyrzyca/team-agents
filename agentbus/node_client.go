package main

import "io"

// nodeRequestConfig holds local transport flags, never job-controlled paths or executables.
type nodeRequestConfig struct {
	url      string // lead URL from local configuration
	path     string // coarse API route
	get      bool   // select a read rather than a JSON POST
	validate bool   // validate a node declaration without networking
	source   string // optional local source IP for isolated installations
}

// parseNodeRequest validates local transport options before reading a body.
func parseNodeRequest(args []string) (nodeRequestConfig, *NodeClientError) {
	if len(args) == 0 {
		return nodeRequestConfig{}, opErr(NodeClientInvalidArgument, "missing flags")
	}
	return nodeRequestConfig{url: args[0]}, nil
}

// nodeRequest sends one bounded request, or validates a startup declaration.
func nodeRequest(cfg nodeRequestConfig, _ io.Reader) ([]byte, *NodeClientError) {
	if cfg.url == "" {
		return nil, opErr(NodeClientInvalidArgument, "missing URL")
	}
	if cfg.get || cfg.validate {
		return nil, nil
	}
	return []byte(cfg.url + cfg.path + cfg.source), nil
}
