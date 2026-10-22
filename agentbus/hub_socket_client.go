// Lead-local clients use the hub daemon's private Unix socket with the same HTTP protocol.
package main

import (
	"context"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

// newHubSocketClient constructs a local transport; a missing socket is an outage, never a file-bus fallback.
func newHubSocketClient(path, instance string) (*HubClient, *HubClientError) {
	if path == "" || !filepath.IsAbs(path) || instance == "" {
		return nil, opErr(HubClientInvalidConfig, "hub socket requires an absolute path and INSTANCE_NAME")
	}
	transport := &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			// DialContext requires error; failures contain *ValueError or *net.OpError.
			if err := checkHubSocket(path); err != nil {
				return nil, err
			}
			var dialer net.Dialer
			return dialer.DialContext(ctx, "unix", path)
		},
		ResponseHeaderTimeout: 30 * time.Second,
		IdleConnTimeout:       60 * time.Second,
	}
	return &HubClient{URL: "http://local-hub", Instance: instance, HTTP: &http.Client{Transport: transport}}, nil
}

// checkHubSocket prevents other local users from supplying a replacement hub endpoint.
// It implements the transport's filesystem admission; failures are reported through DialContext.
func checkHubSocket(path string) *ValueError {
	info, err := os.Lstat(path)
	if err != nil {
		return valueWrap(err, "cannot inspect local hub socket")
	}
	owner, ok := info.Sys().(*syscall.Stat_t)
	if info.Mode()&os.ModeSocket == 0 || info.Mode().Perm()&0o077 != 0 || !ok || int64(owner.Uid) != int64(os.Geteuid()) {
		return valueErr("local hub socket must be owned by this user and accessible only to its owner")
	}
	parent, err := os.Stat(filepath.Dir(path))
	if err != nil {
		return valueWrap(err, "cannot inspect local hub socket directory")
	}
	if parent.Mode().Perm()&0o022 != 0 {
		return valueErr("local hub socket directory must not be writable by other users")
	}
	return nil
}
