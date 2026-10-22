// hub_serve implements the multi-instance hub HTTP listener subcommand (agentbus hub).
package main

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// HubConfig holds configuration options for starting the multi-instance hub HTTP listener.
type HubConfig struct {
	// Dir is the bus state directory path.
	Dir string
	// Listen is the listen address or "tailscale".
	Listen string
	// Port is the TCP port on which the hub listens.
	Port int
	// Auth is the authentication mode ("tailscale" or "token").
	Auth HubAuthMode
	// AllowedInstances is the list of allowed node names/tags (tailscale) or name=addr pairs (token).
	AllowedInstances []string
	// TLSCertFile is the path to the hub's TLS certificate file (token mode).
	TLSCertFile string
	// TLSKeyFile is the path to the hub's TLS private key file (token mode).
	TLSKeyFile string
	// TailscaleSocket is an optional path override to the tailscaled LocalAPI socket.
	TailscaleSocket string
	// SocketPath is the path to the local Unix domain socket for lead-local agents.
	SocketPath string
	// InstanceName is the local instance name (defaults to INSTANCE_NAME or "lead").
	InstanceName string
	// Tokens maps instance name -> pre-shared bearer token (token mode).
	Tokens map[string]string
	// Stop is an optional channel to trigger graceful shutdown (for testing).
	Stop <-chan struct{}
	// Listener is an optional pre-configured net.Listener for TCP (for testing).
	Listener net.Listener
	// UnixListener is an optional pre-configured net.Listener for Unix socket (for testing).
	UnixListener net.Listener
	// DisableSocket disables the Unix domain socket listener (for testing).
	DisableSocket bool
}

// defaultRunResolver creates a run resolver returning a server template matching the caller identity.
func defaultRunResolver(b *Bus) func(HubCaller) (*Server, *HubCallError) {
	return func(c HubCaller) (*Server, *HubCallError) {
		s := &Server{
			bus:  b,
			me:   c.Agent + "@" + c.Instance,
			info: Presence{Agent: c.Agent, Task: c.Task},
			team: c.Team,
			tier: c.Tier,
			role: c.Role,
		}
		return s, nil
	}
}

// splitAllowedInstances splits a comma- or whitespace-delimited string of allowed instances into a slice.
func splitAllowedInstances(raw string) []string {
	parts := strings.FieldsFunc(raw, func(r rune) bool {
		return r == ' ' || r == ',' || r == '\t' || r == '\n'
	})
	var result []string
	for _, p := range parts {
		trimmed := strings.TrimSpace(p)
		if trimmed != "" {
			result = append(result, trimmed)
		}
	}
	return result
}

// findTailscaleIP searches local network interfaces for an IPv4 address in the 100.64.0.0/10 CGNAT range.
func findTailscaleIP() (net.IP, error) {
	_, tsNet, err := net.ParseCIDR("100.64.0.0/10")
	if err != nil {
		return nil, err
	}
	ifaces, err := net.Interfaces()
	if err != nil {
		return nil, fmt.Errorf("cannot list network interfaces: %w", err)
	}
	for _, iface := range ifaces {
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		for _, addr := range addrs {
			var ip net.IP
			switch v := addr.(type) {
			case *net.IPNet:
				ip = v.IP
			case *net.IPAddr:
				ip = v.IP
			}
			if ip != nil && ip.To4() != nil && tsNet.Contains(ip) {
				return ip, nil
			}
		}
	}
	return nil, errors.New("no active tailscale IPv4 address (100.64.0.0/10) found on local interfaces")
}

// parseHubConfig parses command line flags and environment variables into a HubConfig.
func parseHubConfig(args []string) *HubConfig {
	fs := newFlagSet("hub")
	dir := fs.String("dir", defaultDir(), "bus state directory")
	listen := fs.String("listen", "", "listen address (addr:port, or 'tailscale')")
	port := fs.Int("port", 0, "hub listen port (default 7777)")
	auth := fs.String("auth", "", "authentication mode: 'tailscale' (default) or 'token'")
	allowed := fs.String("allowed", "", "allowed instances (space- or comma-separated)")
	certFile := fs.String("cert", "", "TLS certificate file path (token mode)")
	keyFile := fs.String("key", "", "TLS private key file path (token mode)")
	socket := fs.String("socket", "", "tailscale LocalAPI socket path override")
	socketListen := fs.String("socket-listen", "", "local Unix domain socket path for lead agents")
	instanceName := fs.String("instance", "", "local instance name")
	parseFlags(fs, args)

	cfg := &HubConfig{
		Dir:             *dir,
		Listen:          *listen,
		Port:            *port,
		TLSCertFile:     *certFile,
		TLSKeyFile:      *keyFile,
		TailscaleSocket: *socket,
		SocketPath:      *socketListen,
		InstanceName:    *instanceName,
		Tokens:          make(map[string]string),
	}

	if cfg.Listen == "" {
		cfg.Listen = os.Getenv("HUB_LISTEN")
		if cfg.Listen == "" {
			cfg.Listen = "tailscale"
		}
	}

	if cfg.Port <= 0 {
		if pStr := os.Getenv("HUB_PORT"); pStr != "" {
			if p, err := strconv.Atoi(pStr); err == nil {
				cfg.Port = p
			}
		}
		if cfg.Port <= 0 {
			cfg.Port = 7777
		}
	}

	authModeStr := *auth
	if authModeStr == "" {
		authModeStr = os.Getenv("HUB_AUTH")
		if authModeStr == "" {
			authModeStr = "tailscale"
		}
	}
	cfg.Auth = HubAuthMode(strings.ToLower(strings.TrimSpace(authModeStr)))

	allowedStr := *allowed
	if allowedStr == "" {
		allowedStr = os.Getenv("ALLOWED_INSTANCES")
	}
	cfg.AllowedInstances = splitAllowedInstances(allowedStr)

	if cfg.TLSCertFile == "" {
		cfg.TLSCertFile = os.Getenv("HUB_TLS_CERT")
	}
	if cfg.TLSKeyFile == "" {
		cfg.TLSKeyFile = os.Getenv("HUB_TLS_KEY")
	}
	if cfg.TailscaleSocket == "" {
		cfg.TailscaleSocket = os.Getenv("TAILSCALE_SOCKET")
	}

	return cfg
}

// serveHub validates configuration, initializes authentication and bus, binds listener, and serves requests.
func serveHub(cfg *HubConfig) *HubServeError {
	if cfg.Port <= 0 || cfg.Port > 65535 {
		return opErr(HubServeInvalidConfig, "invalid HUB_PORT %d: port must be between 1 and 65535", cfg.Port)
	}

	if cfg.Auth != HubAuthModeTailscale && cfg.Auth != HubAuthModeToken {
		return opErr(HubServeInvalidConfig, "invalid HUB_AUTH %q: must be 'tailscale' or 'token'", cfg.Auth)
	}

	if len(cfg.AllowedInstances) == 0 {
		return opErr(HubServeInvalidConfig, "ALLOWED_INSTANCES cannot be empty")
	}

	if cfg.Dir == "" {
		cfg.Dir = defaultDir()
	}
	if err := os.MkdirAll(cfg.Dir, 0o750); err != nil {
		return fromStorage(HubServeStorage, storageErr(err, "cannot create hub state directory %s", cfg.Dir))
	}
	replayDir := filepath.Join(cfg.Dir, "replay")
	if err := os.MkdirAll(replayDir, 0o750); err != nil {
		return fromStorage(HubServeStorage, storageErr(err, "cannot create replay ledger directory %s", replayDir))
	}

	b := &Bus{dir: cfg.Dir}
	authenticator := NewHubAuthenticator(cfg.Auth)

	switch cfg.Auth {
	case HubAuthModeTailscale:
		authenticator.Tailscale.SocketPath = cfg.TailscaleSocket
		authenticator.Tailscale.AllowedInstances = cfg.AllowedInstances

	case HubAuthModeToken:
		if cfg.TLSCertFile == "" || cfg.TLSKeyFile == "" {
			return opErr(HubServeInvalidConfig, "HUB_TLS_CERT and HUB_TLS_KEY are required in token mode")
		}
		for _, item := range cfg.AllowedInstances {
			parts := strings.SplitN(item, "=", 2)
			if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
				return opErr(HubServeInvalidConfig, "invalid token mode instance definition %q: expected name=address", item)
			}
			instName := strings.TrimSpace(parts[0])
			instAddr := strings.TrimSpace(parts[1])
			token := cfg.Tokens[instName]
			if token == "" {
				envKey := "HUB_TOKEN_" + strings.ReplaceAll(instName, "-", "_")
				token = os.Getenv(envKey)
			}
			if token == "" {
				return opErr(HubServeInvalidConfig, "missing bearer token for instance %q (set HUB_TOKEN_%s)", instName, strings.ReplaceAll(instName, "-", "_"))
			}
			authenticator.Token.Tokens[instName] = token
			authenticator.Token.AllowedAddrs[instName] = instAddr
		}
	}

	hub := &Hub{
		Bus:          b,
		Authenticate: authenticator.Authenticate,
		Resolve:      defaultRunResolver(b),
	}

	ln := cfg.Listener
	if ln == nil {
		listenTarget := cfg.Listen
		if listenTarget == "" || listenTarget == "tailscale" {
			tsIP, err := findTailscaleIP()
			if err != nil {
				return opErr(HubServeListen, "failed to find tailscale IP: %v", err)
			}
			listenTarget = fmt.Sprintf("%s:%d", tsIP.String(), cfg.Port)
		} else if !strings.Contains(listenTarget, ":") {
			listenTarget = fmt.Sprintf("%s:%d", listenTarget, cfg.Port)
		}

		rawListener, err := net.Listen("tcp", listenTarget)
		if err != nil {
			return opErr(HubServeListen, "failed to listen on %s: %v", listenTarget, err)
		}
		ln = rawListener
	}

	if cfg.Auth == HubAuthModeToken {
		tlsCert, err := tls.LoadX509KeyPair(cfg.TLSCertFile, cfg.TLSKeyFile)
		if err != nil {
			if closeErr := ln.Close(); closeErr != nil {
				warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
			}
			return opErr(HubServeListen, "failed to load TLS certificate and key (%s, %s): %v", cfg.TLSCertFile, cfg.TLSKeyFile, err)
		}
		tlsConfig := &tls.Config{
			Certificates: []tls.Certificate{tlsCert},
			MinVersion:   tls.VersionTLS12,
		}
		ln = tls.NewListener(ln, tlsConfig)
	}

	server := &http.Server{
		Handler:           hub,
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       120 * time.Second,
	}

	errCh := make(chan error, 2)
	go func() {
		if err := server.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
	}()

	var unixLn net.Listener
	var socketPath string
	var unixServer *http.Server
	if !cfg.DisableSocket {
		socketPath = cfg.SocketPath
		if socketPath == "" {
			socketPath = os.Getenv("HUB_SOCKET")
			if socketPath == "" {
				socketPath = filepath.Join(cfg.Dir, "hub.sock")
			}
		}
		socketPath = filepath.Clean(socketPath)
		if cfg.UnixListener != nil {
			unixLn = cfg.UnixListener
		} else {
			if err := os.Remove(socketPath); err != nil && !errors.Is(err, os.ErrNotExist) { //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
				if closeErr := ln.Close(); closeErr != nil {
					warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
				}
				return opErr(HubServeSocket, "cannot remove stale unix socket %s: %v", socketPath, err)
			}
			ul, err := net.Listen("unix", socketPath)
			if err != nil {
				if closeErr := ln.Close(); closeErr != nil {
					warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
				}
				return opErr(HubServeSocket, "failed to listen on unix socket %s: %v", socketPath, err)
			}
			if err := os.Chmod(socketPath, 0o600); err != nil { //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
				_ = ul.Close()
				_ = os.Remove(socketPath) //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
				if closeErr := ln.Close(); closeErr != nil {
					warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
				}
				return opErr(HubServeSocket, "failed to set owner-only permissions on unix socket %s: %v", socketPath, err)
			}
			unixLn = ul
		}

		localInstance := cfg.InstanceName
		if localInstance == "" {
			localInstance = os.Getenv("INSTANCE_NAME")
			if localInstance == "" {
				localInstance = "lead"
			}
		}
		unixHub := &Hub{
			Bus: b,
			Authenticate: func(*http.Request) (string, *HubAuthError) {
				return localInstance, nil
			},
			Resolve:      defaultRunResolver(b),
			PollDuration: hub.PollDuration,
		}
		unixServer = &http.Server{
			Handler:           unixHub,
			ReadHeaderTimeout: 10 * time.Second,
			IdleTimeout:       120 * time.Second,
		}
		go func() {
			if err := unixServer.Serve(unixLn); err != nil && !errors.Is(err, http.ErrServerClosed) {
				errCh <- err
			}
		}()
	}

	logInfo(LogHubStarted, "auth", cfg.Auth, "listen", ln.Addr().String(), "socket", socketPath)

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP)

	select {
	case err := <-errCh:
		if err != nil {
			return opErr(HubServeListen, "hub server terminated unexpectedly: %v", err)
		}
	case sig := <-sigCh:
		logInfo(LogHubStopping, "signal", sig.String())
	case <-cfg.Stop:
		logInfo(LogHubStopping, "signal", "stopped")
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := server.Shutdown(ctx); err != nil {
		warnSideEffect("shutdown hub server", storageErr(err, "hub server shutdown failed"))
	}
	if unixServer != nil {
		if err := unixServer.Shutdown(ctx); err != nil {
			warnSideEffect("shutdown hub unix socket server", storageErr(err, "unix server shutdown failed"))
		}
		if cfg.UnixListener == nil && socketPath != "" {
			_ = os.Remove(socketPath) //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
		}
	}

	return nil
}

// runHub parses command line flags and environment variables, then starts the hub HTTP listener.
func runHub(args []string) *HubServeError {
	cfg := parseHubConfig(args)
	return serveHub(cfg)
}

// cmdHub is the agentbus hub subcommand entry point.
func cmdHub(args []string) {
	exitOn(runHub(args).asError())
}
