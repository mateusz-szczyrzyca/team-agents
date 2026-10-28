package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"
)

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
	fs := newFlagSet("node-request")
	base := fs.String("url", "", "lead HTTP URL from local configuration")
	path := fs.String("path", "/v1/heartbeat", "coarse API route")
	get := fs.Bool("get", false, "read a job or node view")
	validate := fs.Bool("validate", false, "validate a startup NodeHeartbeat without connecting")
	source := fs.String("source", "", "optional local source IP")
	if err := fs.Parse(args); err != nil {
		return nodeRequestConfig{}, opErr(NodeClientInvalidArgument, "%v", err)
	}
	u, err := url.Parse(*base)
	if fs.NArg() != 0 || err != nil || u == nil || u.Scheme != "http" || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || u.Opaque != "" || (u.Path != "" && u.Path != "/") || u.RawPath != "" {
		return nodeRequestConfig{}, opErr(NodeClientInvalidArgument, "--url must be a plain HTTP lead origin without credentials, query or path")
	}
	if *source != "" && net.ParseIP(*source) == nil {
		return nodeRequestConfig{}, opErr(NodeClientInvalidArgument, "--source must be a local IP literal")
	}
	if !nodeRouteAllowed(*path, *get) {
		return nodeRequestConfig{}, opErr(NodeClientInvalidArgument, "use a heartbeat, job submission/state, or job/node read route")
	}
	return nodeRequestConfig{url: strings.TrimSuffix(*base, "/"), path: *path, get: *get, validate: *validate, source: *source}, nil
}

// nodeRouteAllowed excludes arbitrary URLs, traversal and operations not available to a node.
func nodeRouteAllowed(path string, get bool) bool {
	if get && (path == "/v1/jobs" || path == "/v1/nodes") {
		return true
	}
	if !get && (path == "/v1/heartbeat" || path == "/v1/jobs" || path == "/v1/sync") {
		return true
	}
	rest, ok := strings.CutPrefix(path, "/v1/jobs/")
	if !ok {
		return false
	}
	if !get {
		var found bool // state reports are the only per-job node mutation
		rest, found = strings.CutSuffix(rest, "/state")
		if !found {
			return false
		}
	}
	return validJobIdentifier(rest)
}

// nodeRequest sends one bounded request, or validates a startup declaration.
func nodeRequest(cfg nodeRequestConfig, input io.Reader) ([]byte, *NodeClientError) {
	if cfg.get && !cfg.validate {
		input = strings.NewReader("")
	}
	requestLimit := 256 * 1024 // ordinary lead request envelope, including JSON escaping
	if cfg.path == "/v1/sync" {
		requestLimit = syncRequestBytes
	}
	data, err := io.ReadAll(io.LimitReader(input, int64(requestLimit)+1))
	if err != nil {
		return nil, opErr(NodeClientInvalidArgument, "cannot read request: %v", err)
	}
	if len(data) > requestLimit {
		return nil, opErr(NodeClientInvalidArgument, "request exceeds %d bytes; send smaller batches", requestLimit)
	}
	if !cfg.get {
		trimmed := bytes.TrimSpace(data)
		if len(trimmed) == 0 || trimmed[0] != '{' || !json.Valid(trimmed) {
			return nil, opErr(NodeClientInvalidArgument, "request must be one JSON object")
		}
	}
	if cfg.validate {
		var beat NodeHeartbeat // startup declaration is validated independently of lead reachability
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&beat); err != nil {
			return nil, opErr(NodeClientInvalidArgument, "invalid node declaration: %v", err)
		}
		if _, err := newLeadRegistry().heartbeat(beat, Timestamp{time.Now().UTC()}); err != nil {
			return nil, opErr(NodeClientInvalidArgument, "invalid node declaration: %v", err)
		}
		return []byte("{}"), nil
	}
	method := http.MethodPost
	if cfg.get {
		method = http.MethodGet
		data = nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, method, cfg.url+cfg.path, bytes.NewReader(data))
	if err != nil {
		return nil, opErr(NodeClientInvalidArgument, "invalid lead request: %v", err)
	}
	req.Header.Set("Content-Type", "application/json")
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	if cfg.source != "" {
		dialer.LocalAddr = &net.TCPAddr{IP: net.ParseIP(cfg.source)}
	}
	transport := &http.Transport{DialContext: dialer.DialContext, ResponseHeaderTimeout: 5 * time.Second}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }} // http.Client requires the plain interface error.
	response, err := client.Do(req)
	if err != nil {
		return nil, opErr(NodeClientUnreachable, "cannot reach lead: %v", err)
	}
	defer func() {
		if err := response.Body.Close(); err != nil {
			warnSideEffect("close node response", storageErr(err, "cannot close response body"))
		}
	}()
	const replyLimit = 1024 * 1024 // bounded assignment plus stop-request reply
	out, err := io.ReadAll(io.LimitReader(response.Body, replyLimit+1))
	if err != nil {
		return nil, opErr(NodeClientUnreachable, "cannot read lead reply: %v", err)
	}
	if response.StatusCode >= 500 {
		return nil, opErr(NodeClientServerError, "lead HTTP %d: %.1000s", response.StatusCode, out)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		if response.StatusCode == http.StatusBadRequest && cfg.path == "/v1/sync" {
			return nil, opErr(NodeClientInvalidArgument, "lead HTTP %d: %.1000s", response.StatusCode, out)
		}
		return nil, opErr(NodeClientRejected, "lead HTTP %d: %.1000s", response.StatusCode, out)
	}
	if len(out) > replyLimit || !json.Valid(out) {
		return nil, opErr(NodeClientMalformedReply, "lead reply must be JSON of at most 1048576 bytes")
	}
	return out, nil
}

// cmdNodeRequest adapts bounded JSON stdin/stdout to the node transport's own typed errors.
func cmdNodeRequest(args []string) {
	cfg, err := parseNodeRequest(args)
	if err != nil {
		exitOn(err.asError())
		return
	}
	out, err := nodeRequest(cfg, os.Stdin)
	if err != nil {
		exitOn(err.asError())
		return
	}
	if _, err := os.Stdout.Write(out); err != nil {
		exitOn(opErr(NodeClientOutputFailed, "cannot write node reply: %v", err).asError())
	}
}
