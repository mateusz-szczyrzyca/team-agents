// The node daemon connects outward to the hub and supervises locally authorized command execution.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// NodeExecutor performs only fixed internal agent-script operations on verified local tasks.
type NodeExecutor interface {
	// Execute reconciles an accepted command after restart when uncertain is true; it must never blindly respawn it.
	Execute(context.Context, Command, bool) (NodeExecutionResult, *ProcessError)
	// Close terminates supervised child process groups during daemon shutdown.
	Close() *ProcessError
}

// NodeConfig holds operator-owned settings, never values selecting a remote executable.
type NodeConfig struct {
	Home              string                                   // local AGENTS_HOME; contains only this node's journal and tasks
	Instance          string                                   // unique qualified-name suffix
	Lead              string                                   // hub origin or host:port; empty/self uses a configured lead-local socket
	Auth              HubAuthMode                              // transport authentication mode
	Token             string                                   // secret from environment, never a command-line argument
	Fingerprint       string                                   // pinned token-mode certificate SHA-256
	AgentCmd          string                                   // fixed local script executable
	Declared          []Capability                             // validated local capability declarations
	AcceptRepos       []string                                 // additional local repository allowlist passed to fixed script admission
	HeartbeatInterval time.Duration                            // zero uses 60 seconds
	RetryInterval     time.Duration                            // zero uses bounded one-second reconnect backoff
	HubRetryDuration  time.Duration                            // maximum control request outage budget; zero uses 30 seconds
	Client            *HubClient                               // optional preconfigured transport for embedding and isolated tests
	Manifest          func() (InstanceManifest, *ProcessError) // optional report provider; no hub lock is held while it runs
	Executor          NodeExecutor                             // optional checked local command executor
}

// Node owns one durable local session; the mutex protects its state, never network calls or child waits.
type Node struct {
	Config   NodeConfig    // immutable after Run starts
	mu       sync.Mutex    // protects state and report sequencing
	state    *nodeState    // loaded after acquiring the lifetime daemon lock
	client   *HubClient    // immutable authenticated HTTP transport
	executor NodeExecutor  // immutable checked local dispatcher
	renew    chan struct{} // coalesces stale-lease notices from the command loop
}

// newNodeClient applies TLS pinning and refuses secret-bearing plaintext transport and redirects.
func newNodeClient(cfg NodeConfig) (*HubClient, *CmdNodeError) {
	if cfg.Client != nil {
		if cfg.Client.Instance != cfg.Instance {
			return nil, opErr(CmdNodeInvalidConfig, "client instance must match node identity")
		}
		return cfg.Client, nil
	}
	mode := cfg.Auth
	if mode == "" {
		mode = HubAuthModeTailscale
	}
	if mode != HubAuthModeTailscale && mode != HubAuthModeToken {
		return nil, opErr(CmdNodeInvalidConfig, "HUB_AUTH must be tailscale or token")
	}
	if (cfg.Lead == "" || cfg.Lead == cfg.Instance) && (os.Getenv("HUB_LISTEN") != "" || os.Getenv("HUB_SOCKET") != "") {
		path := os.Getenv("HUB_SOCKET")
		if path == "" {
			path = filepath.Join(cfg.Home, "bus", "hub.sock")
		}
		client, err := newHubSocketClient(path, cfg.Instance)
		if err != nil {
			return nil, opErr(CmdNodeInvalidConfig, "%s", err.Message)
		}
		client.RetryDuration = cfg.HubRetryDuration
		return client, nil
	}
	transport := &http.Transport{DialContext: (&net.Dialer{Timeout: 5 * time.Second}).DialContext, TLSHandshakeTimeout: 5 * time.Second, ResponseHeaderTimeout: 30 * time.Second, IdleConnTimeout: 60 * time.Second}
	client := &HubClient{URL: cfg.Lead, Instance: cfg.Instance, HTTP: &http.Client{Transport: transport}, RetryDuration: cfg.HubRetryDuration}
	scheme := "http://"
	if mode == HubAuthModeToken {
		if cfg.Token == "" {
			return nil, opErr(CmdNodeInvalidConfig, "token mode requires HUB_TOKEN from secrets.env")
		}
		tlsConfig, err := NewPinnedTLSConfig(cfg.Fingerprint)
		if err != nil {
			return nil, opErr(CmdNodeInvalidConfig, "invalid hub certificate pin: %v", err)
		}
		transport.TLSClientConfig = tlsConfig
		client.Token = cfg.Token
		scheme = "https://"
	}
	if !strings.Contains(client.URL, "://") {
		client.URL = scheme + client.URL
	}
	if err := client.validate(); err != nil {
		return nil, opErr(CmdNodeInvalidConfig, "%s", err.Message)
	}
	return client, nil
}

// validateNodeConfig rejects malformed identity, timeouts and unknown declarations before creating local state.
func validateNodeConfig(cfg NodeConfig) *CmdNodeError {
	if !validInstanceName(cfg.Instance) || cfg.Home == "" || !filepath.IsAbs(cfg.Home) || cfg.HeartbeatInterval < 0 || cfg.RetryInterval < 0 || cfg.HubRetryDuration < 0 {
		return opErr(CmdNodeInvalidConfig, "node requires an instance name, absolute home, and nonnegative intervals")
	}
	for _, capability := range cfg.Declared {
		if !capability.Valid() {
			return opErr(CmdNodeInvalidConfig, "unknown capability %q", capability)
		}
	}
	return nil
}

// Run keeps heartbeats independent of the long poll and preserves local work during hub outages.
func (n *Node) Run(ctx context.Context) *CmdNodeError {
	if err := validateNodeConfig(n.Config); err != nil {
		return err
	}
	client, err := newNodeClient(n.Config)
	if err != nil {
		return err
	}
	n.client = client
	lock, err := lockNodeHome(n.Config.Home)
	if err != nil {
		return err
	}
	defer closeHubFile(lock)
	state, serr := loadNodeState(n.Config.Home, n.Config.Instance)
	if serr != nil {
		return fromStorage(CmdNodeStorage, serr)
	}
	n.state = state
	n.renew = make(chan struct{}, 1)
	if n.state.RegistrationID == "" {
		var session [32]byte
		if _, err := rand.Read(session[:]); err != nil {
			return fromStorage(CmdNodeStorage, storageErr(err, "cannot create node session identity"))
		}
		n.state.RegistrationID = hex.EncodeToString(session[:])
		if serr := saveNodeState(n.Config.Home, n.state); serr != nil {
			return fromStorage(CmdNodeStorage, serr)
		}
	}
	n.executor = n.Config.Executor
	if n.executor == nil {
		n.executor = &nodeProcessExecutor{Home: n.Config.Home, AgentCmd: n.Config.AgentCmd, Instance: n.Config.Instance, Capabilities: n.Config.Declared, AcceptRepos: n.Config.AcceptRepos}
	}
	defer func() {
		if err := n.executor.Close(); err != nil {
			warnSideEffect("close node executor", err)
		}
	}()
	if err := n.registerUntilReady(ctx); err != nil {
		return err
	}
	if ctx.Err() != nil {
		return nil
	}
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	heartbeatDone := make(chan *CmdNodeError, 1)
	go func() {
		failed := n.heartbeatLoop(runCtx)
		if failed != nil {
			cancel()
		}
		heartbeatDone <- failed
	}()
	failed := n.commandLoop(runCtx)
	cancel()
	heartbeatFailure := <-heartbeatDone
	if heartbeatFailure != nil {
		return heartbeatFailure
	}
	return failed
}

// retryPause is cancelable and never holds a node or bus lock.
func (n *Node) retryPause(ctx context.Context) bool {
	interval := n.Config.RetryInterval
	if interval <= 0 {
		interval = time.Second
	}
	timer := time.NewTimer(interval)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-timer.C:
		return true
	}
}

// report generates fresh capabilities and capacity; a configured report provider may supply task summaries.
func (n *Node) report() (InstanceManifest, *CmdNodeError) {
	var manifest InstanceManifest
	if n.Config.Manifest != nil {
		var err *ProcessError
		manifest, err = n.Config.Manifest()
		if err != nil {
			return manifest, opErr(CmdNodeDispatch, "cannot collect node manifest: %s", err.Message)
		}
	} else {
		current, err := nodeCurrentState(n.Config.AgentCmd, n.Config.Home)
		if err != nil {
			return manifest, opErr(CmdNodeDispatch, "cannot collect local node state: %s", err.Message)
		}
		manifest = InstanceManifest{Detected: DetectCapabilities(), CurrentState: current}
	}
	manifest.InstanceName = n.Config.Instance
	manifest.Timestamp = time.Now().UTC()
	manifest.TTLSeconds = int(hubInstanceTTL / time.Second)
	manifest.Declared = append([]Capability(nil), n.Config.Declared...)
	if err := validateHubManifest(manifest); err != nil {
		return manifest, opErr(CmdNodeProtocol, "invalid local manifest: %s", err.Message)
	}
	return manifest, nil
}

// nextReport persists its sequence before sending, so a reconnect cannot replace a newer report.
func (n *Node) nextReport() (InstanceManifest, uint64, HubInstanceLease, *CmdNodeError) {
	manifest, err := n.report()
	if err != nil {
		return manifest, 0, HubInstanceLease{}, err
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	n.state.Sequence++
	if n.state.Sequence == 0 {
		return manifest, 0, HubInstanceLease{}, opErr(CmdNodeProtocol, "node manifest sequence exhausted")
	}
	if serr := saveNodeState(n.Config.Home, n.state); serr != nil {
		return manifest, 0, HubInstanceLease{}, fromStorage(CmdNodeStorage, serr)
	}
	return manifest, n.state.Sequence, n.state.Lease, nil
}

// registerUntilReady retries only availability failures, retaining the durable daemon identity.
func (n *Node) registerUntilReady(ctx context.Context) *CmdNodeError {
	for ctx.Err() == nil {
		manifest, sequence, _, err := n.nextReport()
		if err != nil {
			return err
		}
		n.mu.Lock()
		session := n.state.RegistrationID
		n.mu.Unlock()
		var lease HubInstanceLease
		err = nodeExchange(ctx, n.client, "/v1/instances/register", HubInstanceRegisterRequest{Version: hubProtocolVersion, RegistrationID: session, Sequence: sequence, Manifest: manifest}, &lease)
		if ctx.Err() != nil {
			return nil
		}
		if err == nil {
			return n.storeLease(lease)
		}
		if err.Cause != CmdNodeHubUnavailable {
			return err
		}
		warnOnce("node-register:"+n.Config.Instance, LogSideEffectFailed, err)
		if !n.retryPause(ctx) {
			return nil
		}
	}
	return nil
}

// storeLease validates credentials before making them available to concurrent command polls.
func (n *Node) storeLease(lease HubInstanceLease) *CmdNodeError {
	if lease.Version != hubProtocolVersion || lease.Token == "" || lease.Generation <= 0 || lease.ExpiresAt.IsZero() {
		return opErr(CmdNodeProtocol, "hub returned an invalid node lease")
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	if lease.Generation < n.state.Lease.Generation {
		return opErr(CmdNodeProtocol, "hub lease generation moved backwards")
	}
	n.state.Lease = lease
	return fromStorage(CmdNodeStorage, saveNodeState(n.Config.Home, n.state))
}

// heartbeatLoop renews leases independently from command delivery and execution.
func (n *Node) heartbeatLoop(ctx context.Context) *CmdNodeError {
	interval := n.Config.HeartbeatInterval
	if interval <= 0 {
		interval = time.Minute
	}
	timer := time.NewTimer(interval)
	defer timer.Stop()
	for {
		forceRegistration := false
		select {
		case <-ctx.Done():
			return nil
		case <-n.renew:
			forceRegistration = true
		case <-timer.C:
		}
		if forceRegistration {
			if err := n.registerUntilReady(ctx); err != nil {
				return err
			}
		} else {
			manifest, seq, lease, err := n.nextReport()
			if err != nil {
				return err
			}
			var renewed HubInstanceLease
			err = nodeExchange(ctx, n.client, "/v1/instances/heartbeat", HubInstanceHeartbeatRequest{Version: hubProtocolVersion, Instance: n.Config.Instance, Token: lease.Token, Generation: lease.Generation, Sequence: seq, Manifest: manifest}, &renewed)
			if ctx.Err() != nil {
				return nil
			}
			if serr := recordHubReachability(n.Config.Home, err == nil || err.Cause != CmdNodeHubUnavailable, time.Now()); serr != nil {
				warnSideEffect("record hub reachability", serr, "instance", n.Config.Instance)
			}
			switch {
			case err == nil:
				if err := n.storeLease(renewed); err != nil {
					return err
				}
			case err.Cause == CmdNodeLeaseLost:
				if err := n.registerUntilReady(ctx); err != nil {
					return err
				}
			case err.Cause == CmdNodeHubUnavailable:
				warnOnce("node-heartbeat:"+n.Config.Instance, LogSideEffectFailed, err)
			default:
				return err
			}
		}
		timer.Reset(interval)
	}
}

// leaseSnapshot keeps credentials and acknowledgment cursor consistent across concurrent heartbeat updates.
func (n *Node) leaseSnapshot() (HubInstanceLease, int64) {
	n.mu.Lock()
	defer n.mu.Unlock()
	return n.state.Lease, n.state.After
}

// requestRenew coalesces stale-lease signals while the independent heartbeat loop reconnects.
func (n *Node) requestRenew() {
	select {
	case n.renew <- struct{}{}:
	default:
	}
}

// commandLoop never advances the cursor until an explicit successful acknowledgment is durable locally.
func (n *Node) commandLoop(ctx context.Context) *CmdNodeError {
	for ctx.Err() == nil {
		lease, after := n.leaseSnapshot()
		if !time.Now().Before(lease.ExpiresAt.Time) {
			n.requestRenew()
			if !n.retryPause(ctx) {
				return nil
			}
			continue
		}
		var batch commandReply
		err := nodeExchange(ctx, n.client, "/v1/commands/poll", commandPollRequest{Version: hubProtocolVersion, Generation: lease.Generation, LeaseToken: lease.Token, After: after}, &batch)
		if ctx.Err() != nil {
			return nil
		}
		if err != nil {
			if err.Cause == CmdNodeLeaseLost {
				n.requestRenew()
			} else if err.Cause != CmdNodeHubUnavailable {
				return err
			}
			warnOnce("node-poll:"+n.Config.Instance, LogSideEffectFailed, err)
			if !n.retryPause(ctx) {
				return nil
			}
			continue
		}
		if batch.Version == 0 && len(batch.Commands) == 0 {
			if !n.retryPause(ctx) {
				return nil
			}
			continue
		} // HTTP 204 has no envelope.
		if batch.Version != hubProtocolVersion || batch.Error != nil || len(batch.Commands) > commandBatchLimit {
			return opErr(CmdNodeProtocol, "invalid command batch")
		}
		previous := after
		for _, command := range batch.Commands {
			if command.Seq <= previous || command.Generation != lease.Generation || !command.Kind.Valid() || !validNodeTaskID(command.Task) {
				return opErr(CmdNodeProtocol, "invalid, unordered, or stale command delivery")
			}
			previous = command.Seq
			current, _ := n.leaseSnapshot()
			if current.Generation != command.Generation || !time.Now().Before(current.ExpiresAt.Time) {
				n.requestRenew()
				break
			}
			result, err := n.execute(ctx, command)
			if err != nil {
				return err
			}
			if err := n.acknowledge(ctx, command, result); err != nil {
				return err
			}
		}
	}
	return nil
}

// execute persists acceptance before dispatch and completion before acknowledging a command.
func (n *Node) execute(ctx context.Context, command Command) (NodeExecutionResult, *CmdNodeError) {
	digest, verr := nodeCommandDigest(command)
	if verr != nil {
		return NodeExecutionResult{}, fromValue(CmdNodeProtocol, verr)
	}
	key := strconv.FormatInt(command.Seq, 10)
	n.mu.Lock()
	record := n.state.Commands[key]
	uncertain := false
	if record != nil {
		if record.Digest != digest {
			n.mu.Unlock()
			return NodeExecutionResult{}, opErr(CmdNodeProtocol, "hub reused a command sequence with a different payload")
		}
		if record.Phase != nodeCommandAccepted {
			result := record.Result
			n.mu.Unlock()
			return result, nil
		}
		uncertain = true
	} else {
		record = &nodeCommandRecord{Digest: digest, Phase: nodeCommandAccepted, Command: command}
		n.state.Commands[key] = record
		if serr := saveNodeState(n.Config.Home, n.state); serr != nil {
			n.mu.Unlock()
			return NodeExecutionResult{}, fromStorage(CmdNodeStorage, serr)
		}
	}
	n.mu.Unlock()
	result, processErr := n.executor.Execute(ctx, command, uncertain)
	if processErr != nil {
		result = NodeExecutionResult{Outcome: CommandOutcomeFailed, Detail: processErr.Message}
	}
	if !result.Outcome.Valid() {
		return NodeExecutionResult{}, opErr(CmdNodeProtocol, "executor returned an invalid command outcome")
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	record.Phase, record.Result = nodeCommandCompleted, result
	if serr := saveNodeState(n.Config.Home, n.state); serr != nil {
		return NodeExecutionResult{}, fromStorage(CmdNodeStorage, serr)
	}
	return result, nil
}

// acknowledge retries a recorded outcome across outages and lease renewal without re-executing its command.
func (n *Node) acknowledge(ctx context.Context, command Command, result NodeExecutionResult) *CmdNodeError {
	for ctx.Err() == nil {
		lease, _ := n.leaseSnapshot()
		var reply commandReply
		err := nodeExchange(ctx, n.client, "/v1/commands/ack", commandAckRequest{Version: hubProtocolVersion, Generation: lease.Generation, LeaseToken: lease.Token, Seq: command.Seq, Outcome: result.Outcome, Detail: result.Detail}, &reply)
		if ctx.Err() != nil {
			return nil
		}
		if err == nil {
			if reply.Version != hubProtocolVersion || reply.Error != nil || len(reply.Commands) != 0 {
				return opErr(CmdNodeProtocol, "invalid command acknowledgment reply")
			}
			n.mu.Lock()
			n.state.Commands[strconv.FormatInt(command.Seq, 10)].Phase = nodeCommandAcknowledged
			if command.Seq > n.state.After {
				n.state.After = command.Seq
			}
			serr := saveNodeState(n.Config.Home, n.state)
			n.mu.Unlock()
			return fromStorage(CmdNodeStorage, serr)
		}
		if err.Cause == CmdNodeLeaseLost {
			n.requestRenew()
		} else if err.Cause != CmdNodeHubUnavailable {
			return err
		}
		warnOnce("node-ack:"+n.Config.Instance, LogSideEffectFailed, err)
		if !n.retryPause(ctx) {
			return nil
		}
	}
	return nil
}

// parseNodeConfig reads exported configuration; the agent script sources agents.conf and secrets.env.
func parseNodeConfig(args []string) (*NodeConfig, *CmdNodeError) {
	fs := newFlagSet("node")
	home := fs.String("home", agentsHome(), "local agents home")
	instance := fs.String("instance", os.Getenv("INSTANCE_NAME"), "unique instance name")
	lead := fs.String("lead", os.Getenv("LEAD_INSTANCE"), "lead instance origin or host:port")
	auth := fs.String("auth", os.Getenv("HUB_AUTH"), "tailscale or token")
	caps := fs.String("capabilities", os.Getenv("INSTANCE_CAPABILITIES"), "comma-separated declared capabilities")
	program := fs.String("agent-cmd", "", "fixed agent script executable")
	pin := fs.String("cert-sha256", "", "pinned hub certificate SHA-256")
	acceptedRepos := fs.String("accept-repos", "", "accepted local repository names")
	parseFlags(fs, args)
	absolute, err := filepath.Abs(*home)
	if err != nil {
		return nil, opErr(CmdNodeInvalidConfig, "cannot resolve node home: %v", err)
	}
	script := *program
	if script == "" {
		executable, err := os.Executable()
		if err != nil {
			return nil, opErr(CmdNodeInvalidConfig, "cannot resolve node executable: %v", err)
		}
		script = filepath.Join(filepath.Dir(executable), "agent")
	}
	fingerprint := *pin
	if fingerprint == "" {
		fingerprint = os.Getenv("HUB_CERT_SHA256")
	}
	if fingerprint == "" {
		fingerprint = os.Getenv("HUB_CERT_FINGERPRINT")
	}
	cfg := &NodeConfig{Home: absolute, Instance: *instance, Lead: *lead, Auth: HubAuthMode(*auth), Token: os.Getenv("HUB_TOKEN"), Fingerprint: fingerprint, AgentCmd: script}
	cfg.AcceptRepos = strings.FieldsFunc(*acceptedRepos, func(c rune) bool { return c == ',' || c == ' ' })
	for _, raw := range strings.FieldsFunc(*caps, func(c rune) bool { return c == ',' || c == ' ' || c == '\n' }) {
		capability, err := ParseCapability(raw)
		if err != nil {
			return nil, opErr(CmdNodeInvalidConfig, "%s", err.Message)
		}
		cfg.Declared = append(cfg.Declared, capability)
	}
	seconds := 30
	if raw := os.Getenv("HUB_RETRY_SECONDS"); raw != "" {
		parsed, parseErr := strconv.Atoi(raw)
		if parseErr != nil || parsed <= 0 || parsed > 600 {
			return nil, opErr(CmdNodeInvalidConfig, "HUB_RETRY_SECONDS must be between 1 and 600")
		}
		seconds = parsed
	}
	cfg.HubRetryDuration = time.Duration(seconds) * time.Second
	return cfg, validateNodeConfig(*cfg)
}

// runNode installs signal cancellation while preserving typed daemon failures.
func runNode(args []string) *CmdNodeError {
	cfg, err := parseNodeConfig(args)
	if err != nil {
		return err
	}
	setupLogging(cfg.Home, "node", "instance", cfg.Instance)
	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer cancel()
	return (&Node{Config: *cfg}).Run(ctx)
}

// cmdNode is the CLI boundary for the node daemon.
func cmdNode(args []string) { exitOn(runNode(args).asError()) }

// validNodeTaskID constrains hub-supplied identifiers before constructing any local path or argument.
func validNodeTaskID(id string) bool {
	if id == "" || len(id) > 200 || id == "." || id == ".." {
		return false
	}
	for _, c := range id {
		if (c < 'a' || c > 'z') && (c < 'A' || c > 'Z') && (c < '0' || c > '9') && c != '-' && c != '_' && c != '.' {
			return false
		}
	}
	return true
}

// String avoids exposing the bearer token through accidental configuration formatting.
func (cfg NodeConfig) String() string { return fmt.Sprintf("node %s at %s", cfg.Instance, cfg.Home) }
