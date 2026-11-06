package main

import "io"

// cmdAck is the acknowledgement signature for the test-first restoration.
func (a *App) cmdAck(_ string, _ io.Writer, _ string) *AckError {
	return nil
}
