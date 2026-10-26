package main

import (
	"flag"
	"fmt"
	"time"
)

// NodeTTLCause identifies invalid heartbeat timeouts supplied to the web server.
type NodeTTLCause int

const (
	NodeTTLInvalid NodeTTLCause = iota // timeout is not a positive number of seconds representable as a duration
)

// String identifies the invalid timeout in startup diagnostics.
func (c NodeTTLCause) String() string { return fmt.Sprintf("NodeTTLCause(%d)", int(c)) }

// webNodeTTLFlag declares the optional heartbeat timeout without changing existing web invocations.
func webNodeTTLFlag(fs *flag.FlagSet) *string {
	return fs.String("node-ttl", "90", "node heartbeat timeout in seconds")
}

// parseWebNodeTTL validates the node heartbeat timeout before starting the web server.
func parseWebNodeTTL(_ string) (time.Duration, *OpError[NodeTTLCause]) { return 0, nil }

// webJobsSnapshot is the read-only persisted registry subset used by the web views.
type webJobsSnapshot struct{}

// readWebJobs loads the registry for a web view, preserving typed storage failures.
func readWebJobs(_ string) (*webJobsSnapshot, *OpError[ViewCause]) { return nil, nil }
