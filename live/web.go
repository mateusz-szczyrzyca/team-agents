// agentlive web — the web interface: the same data as the panel (agent live), as views in a browser, plus
// answering questions. Started by `agent web` (and agentweb.service):
//
//	agentlive web --home ~/agents --bus ~/agents/bus --agent ~/.local/bin/agent --listen 127.0.0.1:8787 --users a@b
//
// It listens on a loopback address only. Remote access goes through `tailscale serve`, which adds the
// Tailscale-User-Login header; only the logins given in --users are let in that way. A request forwarded by a
// proxy without that identity (Tailscale Funnel, another proxy) is refused. Pages are plain HTML from html/template;
// htmx (vendored, see web/static/vendor/SOURCES) swaps the view and keeps live views up to date over server-sent
// events. No JavaScript build, no npm.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"html/template"
	"io"
	"io/fs"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"
)

//go:embed web/templates/*.html web/static
var webFS embed.FS

// webConfig holds the settings of agentlive web.
type webConfig struct {
	config                 // home, bus directory, agent script, queue limit (shared with the panel)
	listen string          // host:port to listen on (loopback only)
	users  map[string]bool // Tailscale logins allowed through `tailscale serve`
	poll   time.Duration   // how often live views re-read the files
}

// webServer serves the web interface.
type webServer struct {
	cfg    webConfig                     // settings
	pages  map[string]*template.Template // view name → layout and view
	frags  *template.Template            // the partials sent by live updates
	static http.Handler                  // the embedded static files
	assetV string                        // version of the static files (cache busting)
	host   string                        // this host's name, shown in the header
	agents agentsCache                   // the configured agents (from `agent _agents-json`)
}

// AgentCallCause: why a call of the `agent` script failed.
type AgentCallCause int

const (
	AgentCallStart AgentCallCause = iota // the script could not be started
	AgentCallExit                        // it exited with a non-zero status
)

// String returns the cause name.
func (c AgentCallCause) String() string {
	switch c {
	case AgentCallStart:
		return "Start"
	case AgentCallExit:
		return "Exit"
	}
	return fmt.Sprintf("AgentCallCause(%d)", int(c))
}

// callAgent runs the `agent` script with arguments and returns its output (stdout and stderr). Tests replace it;
// the test binary's default fails the test, so no test can touch the real installation by accident.
var callAgent = func(agentCmd string, args ...string) (string, *OpError[AgentCallCause]) {
	cmd := exec.Command(agentCmd, args...) // #nosec G204 -- the configured agent script with fixed subcommands
	out, err := cmd.CombinedOutput()
	if err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) {
			return string(out), webErr(AgentCallExit, err, "%s %s: %s", filepath.Base(agentCmd), strings.Join(args, " "), strings.TrimSpace(string(out)))
		}
		return "", webErr(AgentCallStart, err, "cannot run %s", agentCmd)
	}
	return string(out), nil
}

// runWebCmd parses the flags of `agentlive web` and serves until SIGINT/SIGTERM.
func runWebCmd(args []string) {
	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentlive web: no home directory:", err)
		os.Exit(2)
	}
	fs := flag.NewFlagSet("agentlive web", flag.ExitOnError)
	cfg := webConfig{}
	fs.StringVar(&cfg.home, "home", filepath.Join(home, "agents"), "AGENTS_HOME directory")
	fs.StringVar(&cfg.busDir, "bus", "", "bus directory (default <home>/bus)")
	fs.StringVar(&cfg.agentCmd, "agent", filepath.Join(home, ".local", "bin", "agent"), "path to the agent script")
	fs.StringVar(&cfg.listen, "listen", "127.0.0.1:8787", "loopback address to listen on")
	users := fs.String("users", "", "Tailscale logins allowed through `tailscale serve` (comma or space separated)")
	fs.IntVar(&cfg.queueMax, "queue-max", 3, "investigation queue limit")
	fs.IntVar(&InvSlots, "inv-slots", 1, "number of investigators")
	if err := fs.Parse(args); err != nil {
		os.Exit(2)
	}
	if cfg.busDir == "" {
		cfg.busDir = filepath.Join(cfg.home, "bus")
	}
	cfg.users = parseUsers(*users)
	cfg.poll = time.Second
	setupLogging(cfg.home, "web")

	srv, serr := newWebServer(cfg)
	if serr != nil {
		logError(LogWebStartFailed, serr, "cause", serr.Cause.String())
		fmt.Fprintln(os.Stderr, "agentlive web:", serr)
		os.Exit(1)
	}
	ln, lerr := listenLoopback(cfg.listen)
	if lerr != nil {
		logError(LogWebStartFailed, lerr, "cause", lerr.Cause.String())
		fmt.Fprintln(os.Stderr, "agentlive web:", lerr)
		os.Exit(1)
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	// requests derive from ctx: on SIGTERM the live streams end at once instead of holding up the shutdown
	hs := &http.Server{Handler: srv.handler(), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 2 * time.Minute,
		BaseContext: func(net.Listener) context.Context { return ctx }}
	go func() {
		<-ctx.Done()
		sctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := hs.Shutdown(sctx); err != nil {
			logWarn(LogWebStopFailed, "error", err.Error())
		}
	}()
	logInfo(LogWebStarted, "listen", ln.Addr().String(), "users", len(cfg.users))
	if err := hs.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
		logWarn(LogWebStopFailed, "error", err.Error())
		os.Exit(1)
	}
}

// parseUsers splits the --users list.
func parseUsers(s string) map[string]bool {
	out := map[string]bool{}
	for _, u := range strings.FieldsFunc(s, func(r rune) bool { return r == ',' || r == ' ' || r == '\t' }) {
		out[strings.ToLower(u)] = true
	}
	return out
}

// listenLoopback binds a loopback address. Any other address is refused: remote access goes through
// `tailscale serve`, which proxies to the loopback address and tells who the user is.
func listenLoopback(addr string) (net.Listener, *OpError[StartCause]) {
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, webErr(StartBadListen, err, "listen address %q", addr)
	}
	if !isLoopbackHost(host) {
		return nil, webErr(StartNotLoopback, nil, "listen address %q is not a loopback address — use 127.0.0.1 or ::1 and `tailscale serve` for remote access", addr)
	}
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return nil, webErr(StartListen, err, "cannot listen on %s", addr)
	}
	return ln, nil
}

// isLoopbackHost reports whether a host (name or IP, from an address) is a loopback one.
func isLoopbackHost(host string) bool {
	if host == "localhost" {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// newWebServer parses the templates and prepares the static files.
func newWebServer(cfg webConfig) (*webServer, *OpError[StartCause]) {
	s := &webServer{cfg: cfg, pages: map[string]*template.Template{}}
	host, err := os.Hostname()
	if err != nil {
		logWarn(LogWebStartFailed, "error", "cannot read the host name (the header shows none): "+err.Error())
	}
	s.host = host
	s.agents.ttl = 30 * time.Second
	var static fs.FS
	static, err = fs.Sub(webFS, "web/static")
	if err != nil {
		return nil, webErr(StartAssets, err, "embedded static files")
	}
	h := sha256.New()
	if err := fs.WalkDir(static, ".", func(p string, d fs.DirEntry, werr error) error {
		if werr != nil || d.IsDir() {
			return werr
		}
		b, rerr := fs.ReadFile(static, p)
		h.Write(b)
		return rerr
	}); err != nil {
		return nil, webErr(StartAssets, err, "embedded static files")
	}
	s.assetV = hex.EncodeToString(h.Sum(nil))[:12]
	s.static = http.StripPrefix("/static/", http.FileServer(http.FS(static)))
	base, err := template.New("layout.html").Funcs(webFuncs).ParseFS(webFS, "web/templates/layout.html", "web/templates/partials.html")
	if err != nil {
		return nil, webErr(StartAssets, err, "templates")
	}
	s.frags = base
	views, err := fs.Glob(webFS, "web/templates/*.html")
	if err != nil {
		return nil, webErr(StartAssets, err, "templates")
	}
	for _, v := range views {
		name := strings.TrimSuffix(filepath.Base(v), ".html")
		if name == "layout" || name == "partials" {
			continue
		}
		t, err := base.Clone()
		if err == nil {
			t, err = t.ParseFS(webFS, v)
		}
		if err != nil {
			return nil, webErr(StartAssets, err, "template %s", v)
		}
		s.pages[name] = t
	}
	return s, nil
}

// handler returns the routes behind the access check and the security headers.
func (s *webServer) handler() http.Handler {
	mux := http.NewServeMux()
	mux.Handle("GET /static/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "public, max-age=31536000, immutable") // URLs carry ?v=<assets version>
		s.static.ServeHTTP(w, r)
	}))
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) { writeText(w, "ok") })
	mux.HandleFunc("GET /{$}", s.viewDashboard)
	mux.HandleFunc("GET /agents", s.viewAgents)
	mux.HandleFunc("GET /task/{id}", s.viewTask)
	mux.HandleFunc("GET /teams", s.viewTeams)
	mux.HandleFunc("GET /team/{id}", s.viewTeam)
	mux.HandleFunc("GET /bus", s.viewBus)
	mux.HandleFunc("GET /questions", s.viewQuestions)
	mux.HandleFunc("GET /questions/{id}", s.viewAnswer)
	mux.HandleFunc("POST /questions/{id}", s.postAnswer)
	mux.HandleFunc("GET /investigations", s.viewInvestigations)
	mux.HandleFunc("GET /investigation/{id}", s.viewInvestigation)
	mux.HandleFunc("GET /logs", s.viewLogs)
	mux.HandleFunc("GET /events/dashboard", s.streamDashboard)
	mux.HandleFunc("GET /events/task/{id}", s.streamTask)
	mux.HandleFunc("GET /events/bus", s.streamBus)
	return s.guard(mux)
}

// guard checks every request (checkAccess) and sets the security headers.
func (s *webServer) guard(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; "+
			"connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Referrer-Policy", "same-origin")
		h.Set("X-Frame-Options", "DENY")
		if aerr := s.checkAccess(r); aerr != nil {
			logWarn(LogWebAccessDenied, "cause", aerr.Cause.String(), "path", r.URL.Path, "remote", r.RemoteAddr,
				"user", r.Header.Get("Tailscale-User-Login"), "error", aerr.Message)
			http.Error(w, "forbidden: "+aerr.Message, aerr.Cause.HTTPStatus())
			return
		}
		next.ServeHTTP(w, r)
	})
}

// proxyHeaders are the headers a forwarding proxy adds; with them and without a Tailscale identity the request
// came through something other than `tailscale serve` (e.g. Funnel, open to the Internet).
var proxyHeaders = []string{"X-Forwarded-For", "X-Forwarded-Host", "X-Forwarded-Proto", "Forwarded", "Tailscale-Funnel-Request", "X-Real-Ip"}

// checkAccess decides whether a request may be served:
//   - the connection must come from this host (the server listens on loopback; checked again here);
//   - a request with a Tailscale identity (Tailscale-User-Login, set by `tailscale serve`) needs a login from --users;
//   - a proxied request without one is refused;
//   - a direct local request (a browser on this host, an SSH tunnel) is allowed;
//   - a state-changing request (POST) must come from htmx on this site (HX-Request, same origin).
func (s *webServer) checkAccess(r *http.Request) *OpError[AccessCause] {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil || !isLoopbackHost(host) {
		return webErr(AccessNotLoopback, nil, "connections are accepted from this host only")
	}
	if login := strings.ToLower(strings.TrimSpace(r.Header.Get("Tailscale-User-Login"))); login != "" {
		if !s.cfg.users[login] {
			return webErr(AccessUnknownUser, nil, "Tailscale user %s is not allowed (AGENT_WEB_USERS in agents.conf)", login)
		}
	} else {
		for _, ph := range proxyHeaders {
			if r.Header.Get(ph) != "" {
				return webErr(AccessProxiedAnonymous, nil, "a proxied request without a Tailscale identity (Funnel or another proxy) is refused")
			}
		}
	}
	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		if r.Header.Get("HX-Request") != "true" {
			return webErr(AccessNotHTMX, nil, "changes are accepted from this interface only")
		}
		if site := r.Header.Get("Sec-Fetch-Site"); site != "" && site != "same-origin" {
			return webErr(AccessCrossSite, nil, "a request from another site (%s)", site)
		}
		if origin := r.Header.Get("Origin"); origin != "" {
			u, perr := url.Parse(origin)
			if perr != nil || u.Host != r.Host {
				return webErr(AccessCrossSite, nil, "a request from another origin (%s)", origin)
			}
		}
	}
	return nil
}

// page is what the layout renders around a view.
type page struct {
	Title   string // the view's title (and the browser tab's)
	View    string // the view's name (menu highlighting)
	Max     bool   // maximized: the view covers the menu (?max=1)
	Host    string // this host
	AssetV  string // version of the static files
	Waiting int    // tasks waiting for answers (shown in the menu)
	Now     string // when the page was rendered
	Data    any    // the view's data
}

// render writes a full page; htmx picks the view out of it (hx-select), so every URL is also a full page.
func (s *webServer) render(w http.ResponseWriter, r *http.Request, status int, view, title string, snap *Snapshot, data any) {
	t := s.pages[view]
	if t == nil {
		http.Error(w, "unknown view "+view, http.StatusInternalServerError)
		return
	}
	p := page{Title: title, View: view, Max: r.URL.Query().Get("max") == "1", Host: s.host, AssetV: s.assetV,
		Now: time.Now().Format("15:04:05"), Data: data}
	if snap != nil {
		p.Waiting = len(snap.Waiting())
	}
	var buf bytes.Buffer
	if err := t.ExecuteTemplate(&buf, "layout.html", p); err != nil {
		logWarn(LogWebRenderFailed, "view", view, "error", err.Error())
		http.Error(w, "cannot render the page: "+err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Vary", "HX-Request")
	w.WriteHeader(status)
	if _, err := w.Write(buf.Bytes()); err != nil {
		logWarn(LogWebWriteFailed, "view", view, "error", err.Error())
	}
}

// fragment renders a partial template to a string (for live updates).
func (s *webServer) fragment(name string, data any) string {
	var buf bytes.Buffer
	if err := s.frags.ExecuteTemplate(&buf, name, data); err != nil {
		logWarn(LogWebRenderFailed, "fragment", name, "error", err.Error())
		return ""
	}
	return buf.String()
}

// viewError renders an error page for a failed view or answer.
func viewError[C interface {
	webCause
	HTTPStatus() int
}](s *webServer, w http.ResponseWriter, r *http.Request, e *OpError[C]) {
	logWarn(LogWebViewFailed, "path", r.URL.Path, "cause", e.Cause.String(), "error", e.Message)
	s.render(w, r, e.Cause.HTTPStatus(), "error", "Error", nil, map[string]string{"Cause": e.Cause.String(), "Message": e.Message})
}

// writeText writes a plain-text response (health check).
func writeText(w http.ResponseWriter, s string) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	if _, err := io.WriteString(w, s+"\n"); err != nil {
		logWarn(LogWebWriteFailed, "error", err.Error())
	}
}

// ---------------------------------------------------------------- server-sent events

// sseStream prepares a response for server-sent events; ok is false when the connection cannot stream.
func sseStream(w http.ResponseWriter) (http.Flusher, bool) {
	f, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return nil, false
	}
	h := w.Header()
	h.Set("Content-Type", "text/event-stream")
	h.Set("Cache-Control", "no-store")
	h.Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	f.Flush()
	return f, true
}

// sseSend writes one event (data may span lines) and flushes it; false when the client has gone.
func sseSend(w io.Writer, f http.Flusher, event, data string) bool {
	var b strings.Builder
	b.WriteString("event: " + event + "\n")
	for _, l := range strings.Split(data, "\n") {
		b.WriteString("data: " + l + "\n")
	}
	b.WriteString("\n")
	if _, err := io.WriteString(w, b.String()); err != nil {
		return false
	}
	f.Flush()
	return true
}

// ---------------------------------------------------------------- the configured agents

// agentEntry is one configured agent, from `agent _agents-json`.
type agentEntry struct {
	Name     string `json:"name"`     // agent name
	Type     string `json:"type"`     // CLI: claude | codex | gemini | opencode | agy
	Model    string `json:"model"`    // configured model ("" = the CLI's default)
	Tier     string `json:"tier"`     // normal | worker | investigator
	Provider string `json:"provider"` // provider (load balancing)
}

// agentsCache caches the agent list (it comes from the configuration, which changes rarely).
type agentsCache struct {
	mu   sync.Mutex    // guards the fields below
	at   time.Time     // when the list was read
	list []agentEntry  // the agents, sorted by name
	err  string        // the last failure ("" = none)
	ttl  time.Duration // how long a list is reused
}

// get returns the agents (re-read when older than the ttl) and the last failure.
func (c *agentsCache) get(agentCmd string) ([]agentEntry, string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if time.Since(c.at) < c.ttl && c.list != nil {
		return c.list, c.err
	}
	out, aerr := callAgent(agentCmd, "_agents-json")
	c.at = time.Now()
	if aerr != nil {
		logWarn(LogWebAgentsFailed, "cause", aerr.Cause.String(), "error", aerr.Message)
		c.err = aerr.Message
		return c.list, c.err
	}
	list, perr := parseAgents(out)
	if perr != nil {
		logWarn(LogWebAgentsFailed, "error", perr.Error())
		c.err = perr.Error()
		return c.list, c.err
	}
	c.list, c.err = list, ""
	return c.list, ""
}

// parseAgents parses the JSON lines of `agent _agents-json`.
func parseAgents(out string) ([]agentEntry, *ValueError) {
	var list []agentEntry
	for _, l := range strings.Split(strings.TrimSpace(out), "\n") {
		if strings.TrimSpace(l) == "" {
			continue
		}
		var a agentEntry
		if err := json.Unmarshal([]byte(l), &a); err != nil {
			return nil, valueWrap(err, "agent _agents-json: bad line %q", l)
		}
		list = append(list, a)
	}
	sort.Slice(list, func(i, j int) bool { return list[i].Name < list[j].Name })
	return list, nil
}
