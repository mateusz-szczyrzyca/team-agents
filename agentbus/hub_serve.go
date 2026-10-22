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
	// ManifestTTL is the hub-authoritative lease duration; zero uses 180 seconds.
	ManifestTTL time.Duration
	// TasksDir is the lead-local task metadata directory; empty uses the sibling tasks directory.
	TasksDir string
	// Resolve is an optional identity resolver (defaults to trusted instance reports and hub-owned run assignments).
	Resolve func(HubCaller) (*Server, *HubCallError)
	// BackupDir is where periodic state backups are written (HUB_BACKUP_DIR); empty uses bus/backup under Dir.
	BackupDir string
	// BackupInterval is the time between backups (HUB_BACKUP_MINUTES); zero uses 30 minutes, a negative value is invalid.
	BackupInterval time.Duration
}

// defaultRunResolver denies identities until local metadata or authenticated instance reports are configured.
func defaultRunResolver(b *Bus) func(HubCaller) (*Server, *HubCallError) {
	return (hubRunRegistry{bus: b}).resolve
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
func findTailscaleIP() (net.IP, *HubServeError) {
	_, tsNet, err := net.ParseCIDR("100.64.0.0/10")
	if err != nil {
		return nil, opErr(HubServeInvalidConfig, "internal CIDR parse error: %v", err)
	}
	ifaces, err := net.Interfaces()
	if err != nil {
		return nil, fromStorage(HubServeStorage, storageErr(err, "cannot list network interfaces"))
	}
	for _, iface := range ifaces {
		addrs, err := iface.Addrs()
		if err != nil {
			warnSideEffect("list interface addresses", storageErr(err, "cannot read addresses for interface %s", iface.Name))
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
	return nil, opErr(HubServeListen, "no active tailscale IPv4 address (100.64.0.0/10) found on local interfaces")
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
	tasksDir := fs.String("tasks-dir", "", "lead-local task metadata directory")
	manifestTTL := fs.Int("manifest-ttl", 0, "manifest lifetime in seconds (default 180)")
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
		TasksDir:        *tasksDir,
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

	ttlSeconds := *manifestTTL
	if ttlSeconds == 0 {
		if raw := os.Getenv("MANIFEST_TTL"); raw != "" {
			parsed, err := strconv.Atoi(raw)
			if err != nil || parsed <= 0 || parsed > 86400 {
				ttlSeconds = -1
			} else {
				ttlSeconds = parsed
			}
		}
	}
	if ttlSeconds < 0 || ttlSeconds > 86400 {
		cfg.ManifestTTL = -1
	} else {
		cfg.ManifestTTL = time.Duration(ttlSeconds) * time.Second
	}

	cfg.BackupDir = os.Getenv("HUB_BACKUP_DIR")
	cfg.BackupInterval = backupIntervalFromEnv(os.Getenv("HUB_BACKUP_MINUTES"))
	return cfg
}

// backupIntervalFromEnv parses HUB_BACKUP_MINUTES. Unset gives zero (the default); an unusable value gives -1, which
// serveHub rejects with a configuration error.
func backupIntervalFromEnv(raw string) time.Duration {
	if raw == "" {
		return 0
	}
	minutes, err := strconv.Atoi(raw)
	if err != nil || minutes < 1 || minutes > 1440 {
		return -1
	}
	return time.Duration(minutes) * time.Minute
}

// serveHub validates configuration, initializes authentication and bus, binds listener, and serves requests.
func serveHub(cfg *HubConfig) *HubServeError {
	if cfg.ManifestTTL < 0 || cfg.ManifestTTL > 24*time.Hour {
		return opErr(HubServeInvalidConfig, "MANIFEST_TTL must be between 1 and 86400 seconds")
	}
	if cfg.Port <= 0 || cfg.Port > 65535 {
		return opErr(HubServeInvalidConfig, "invalid HUB_PORT %d: port must be between 1 and 65535", cfg.Port)
	}
	if cfg.BackupInterval == 0 {
		cfg.BackupInterval = defaultBackupInterval
	}
	if cfg.BackupInterval < time.Minute || cfg.BackupInterval > 24*time.Hour {
		return opErr(HubServeInvalidConfig, "HUB_BACKUP_MINUTES must be between 1 and 1440")
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

	localInstance := cfg.InstanceName
	if localInstance == "" {
		localInstance = os.Getenv("INSTANCE_NAME")
	}
	if localInstance == "" {
		localInstance = "lead"
	}
	if !validInstanceName(localInstance) {
		return opErr(HubServeInvalidConfig, "invalid local instance name %q", localInstance)
	}
	tasksDir := cfg.TasksDir
	if tasksDir == "" {
		tasksDir = filepath.Join(filepath.Dir(cfg.Dir), "tasks")
	}
	// The lead is an instance too: its declared and detected capabilities are discoverable like the others'.
	leadDirectory, verr := newLocalDirectory(localInstance, os.Getenv("INSTANCE_CAPABILITIES"))
	if verr != nil {
		return fromValue(HubServeInvalidConfig, verr)
	}
	instances := &HubInstanceRegistry{Bus: b, TTL: cfg.ManifestTTL}
	var authorizePeer func(string, string) *HubCallError // custom resolvers own their authentication contract
	resolver := cfg.Resolve
	if resolver == nil {
		runs := hubRunRegistry{bus: b, instances: instances, localInstance: localInstance, localView: &leadDirectory.view, tasksDir: tasksDir}
		resolver = runs.resolve
		authorizePeer = runs.authorizePeer
	}
	hub := &Hub{
		Instances:     instances,
		Bus:           b,
		Authenticate:  authenticator.Authenticate,
		Resolve:       resolver,
		AuthorizePeer: authorizePeer,
		CLI:           hubCLIDispatcher(b, instances),
	}

	// Maintenance runs until this function returns: periodic backups and the reaper of expired instances.
	maintenanceStop := make(chan struct{})
	defer close(maintenanceStop)
	backupDir := cfg.BackupDir
	if backupDir == "" {
		backupDir = filepath.Join(cfg.Dir, "backup")
	}
	go runBackups(b, backupDir, cfg.BackupInterval, maintenanceStop)
	go runReaper(b, instances, maintenanceStop)

	ln := cfg.Listener
	if ln == nil {
		listenTarget := cfg.Listen
		if listenTarget == "" || listenTarget == "tailscale" {
			tsIP, err := findTailscaleIP()
			if err != nil {
				return err
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
			if _, err := os.Stat(socketPath); err == nil {
				// Verify if another daemon is actively listening on this socket before removing
				conn, dialErr := net.DialTimeout("unix", socketPath, 100*time.Millisecond) //nolint:gosec // G704: socketPath is dialed exclusively over unix domain socket
				if dialErr == nil {
					if err := conn.Close(); err != nil {
						warnSideEffect("close active hub socket probe", storageErr(err, "cannot close socket probe"))
					}
					if closeErr := ln.Close(); closeErr != nil {
						warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
					}
					return opErr(HubServeSocket, "another hub daemon is actively listening on unix socket %s", socketPath)
				}
				if err := os.Remove(socketPath); err != nil { //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
					if closeErr := ln.Close(); closeErr != nil {
						warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
					}
					return opErr(HubServeSocket, "cannot remove stale unix socket %s: %v", socketPath, err)
				}
			}
			ul, err := net.Listen("unix", socketPath)
			if err != nil {
				if closeErr := ln.Close(); closeErr != nil {
					warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
				}
				return opErr(HubServeSocket, "failed to listen on unix socket %s: %v", socketPath, err)
			}
			if err := os.Chmod(socketPath, 0o600); err != nil { //nolint:gosec // G703: socketPath is configured by the operator via HUB_SOCKET or defaultDir
				if closeErr := ul.Close(); closeErr != nil {
					warnSideEffect("close hub unix listener", storageErr(closeErr, "cannot close unix listener"))
				}
				removeHubSocket(socketPath)
				if closeErr := ln.Close(); closeErr != nil {
					warnSideEffect("close hub listener", storageErr(closeErr, "cannot close listener"))
				}
				return opErr(HubServeSocket, "failed to set owner-only permissions on unix socket %s: %v", socketPath, err)
			}
			unixLn = ul
		}

		unixHub := &Hub{
			Instances: instances,
			Bus:       b,
			Authenticate: func(r *http.Request) (string, *HubAuthError) {
				setHubAuthenticatedPeer(r, "local:"+localInstance)
				return localInstance, nil
			},
			Resolve:       resolver,
			AuthorizePeer: authorizePeer,
			PollDuration:  hub.PollDuration,
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
			removeHubSocket(socketPath)
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

// removeHubSocket reports cleanup failures; UnixListener.Close may already have unlinked its socket.
func removeHubSocket(path string) {
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) { //nolint:gosec // G703: operator-configured local socket path, never supplied by a remote request
		warnSideEffect("remove hub unix socket", storageErr(err, "cannot remove unix socket %s", path))
	}
}
