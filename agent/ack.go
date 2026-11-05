package main

import "io"

// cmdAck acknowledges terminal tasks in the explicitly supplied task directory.
func (a *App) cmdAck(_ string, _ io.Writer, _ string) *AckError {
	return nil
}
