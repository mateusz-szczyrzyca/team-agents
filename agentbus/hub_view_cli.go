package main

import (
	"context"
	"time"
)

// runSnapshot exposes the same credential-free view locally or through the configured hub transport.
func runSnapshot(args []string) *CmdSnapshotError {
	fs := newFlagSet("snapshot")
	dir := fs.String("dir", defaultDir(), "bus directory or lead-local socket root")
	timeout := fs.Int("timeout", 3, "maximum view read duration in seconds (1..600)")
	parseFlags(fs, args)
	if *timeout < 1 || *timeout > 600 {
		return opErr(CmdSnapshotInvalidConfig, "snapshot --timeout must be between 1 and 600 seconds")
	}
	client, err := hubClientForDir(*dir)
	if err != nil {
		return opErr(CmdSnapshotInvalidConfig, "%s", err.Message)
	}
	var view *HubViewSnapshot
	if client == nil {
		bus := &Bus{dir: *dir}
		var failed *HubViewError
		view, failed = (&Hub{Bus: bus, Instances: &HubInstanceRegistry{Bus: bus}}).viewSnapshot()
		if failed != nil {
			return opErr(CmdSnapshotStorage, "%s", failed.Message)
		}
	} else {
		ctx, cancel := context.WithTimeout(context.Background(), time.Duration(*timeout)*time.Second)
		defer cancel()
		var failed *CmdSnapshotError
		view, failed = client.Snapshot(ctx)
		if failed != nil {
			return failed
		}
	}
	return fromStorage(CmdSnapshotStorage, printJSON(view))
}
