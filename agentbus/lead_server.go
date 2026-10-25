package main

import (
	"context"
	"errors"
	"log"
	"net"
	"net/http"
	"strings"
	"time"
)

// leadHTTPLogWriter routes net/http diagnostics through the same structured event schema as the CLI.
type leadHTTPLogWriter struct{}

// Write implements io.Writer. Logging has no recoverable write error, so the interface error is always nil.
func (leadHTTPLogWriter) Write(p []byte) (int, error) {
	logWarn(LogLeadHTTPError, "error", strings.TrimSpace(string(p)))
	return len(p), nil
}

// runLead opens every configured address before serving, then drains all servers on cancellation or failure.
// A partial bind failure closes earlier listeners; a started server owns and closes its listener.
func runLead(ctx context.Context, cfg leadConfig) *LeadServeError {
	if ctx.Err() != nil {
		return nil
	}
	var listeners []net.Listener // owned here until transferred to http.Server.Serve
	for _, addr := range leadListenAddrs(cfg.listen) {
		listener, err := net.Listen("tcp", addr.String())
		if err != nil {
			for _, opened := range listeners {
				if cerr := opened.Close(); cerr != nil {
					warnSideEffect("close partial lead listener", opErr(LeadServeServeFailed, "%v", cerr))
				}
			}
			return opErr(LeadServeListenFailed, "listen on %s: %v", addr, err)
		}
		listeners = append(listeners, listener)
	}
	handler := newLeadHandler(&Bus{dir: cfg.dir}, cfg.node, cfg.allowed, cfg.ttl)
	servers := make([]*http.Server, 0, len(listeners))
	stopped := make(chan *LeadServeError, len(listeners)) // one completion per server; shutdown cannot block a sender
	for _, listener := range listeners {
		server := &http.Server{Handler: handler, ReadHeaderTimeout: 5 * time.Second, ReadTimeout: 15 * time.Second, WriteTimeout: 15 * time.Second, IdleTimeout: 30 * time.Second, MaxHeaderBytes: 16 * 1024, ErrorLog: log.New(leadHTTPLogWriter{}, "", 0)}
		servers = append(servers, server)
		go func() {
			if err := server.Serve(listener); err != nil && !errors.Is(err, http.ErrServerClosed) {
				stopped <- opErr(LeadServeServeFailed, "serve %s: %v", listener.Addr(), err)
				return
			}
			stopped <- nil
		}()
	}
	logInfo(LogLeadListening, "node", cfg.node, "listen", cfg.listen.String())
	var failure *LeadServeError // first failure remains primary; cleanup failures are logged as side effects
	remaining := len(servers)
	select {
	case <-ctx.Done():
	case failure = <-stopped:
		remaining--
	}
	drain, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	for _, server := range servers {
		if err := server.Shutdown(drain); err != nil {
			failure = keepLeadFailure(failure, opErr(LeadServeServeFailed, "drain lead requests: %v", err))
			// A request that exceeds the bounded grace period must not keep the service alive indefinitely.
			if err := server.Close(); err != nil {
				failure = keepLeadFailure(failure, opErr(LeadServeServeFailed, "close lead server: %v", err))
			}
		}
	}
	for range remaining {
		if err := <-stopped; err != nil {
			failure = keepLeadFailure(failure, err)
		}
	}
	logInfo(LogLeadStopped, "node", cfg.node)
	return failure
}

// keepLeadFailure preserves the primary failure while retaining evidence of a failed cleanup side effect.
func keepLeadFailure(primary, next *LeadServeError) *LeadServeError {
	if primary == nil {
		return next
	}
	warnSideEffect("lead server cleanup", next)
	return primary
}
