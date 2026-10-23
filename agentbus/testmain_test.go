package main

import (
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

// suiteIsolation owns process defaults; individual tests can still install scoped capture hooks.
var suiteIsolation struct {
	root     string     // temporary parent of HOME, AGENTS_HOME and the suite log
	log      *os.File   // suite-owned fallback logger, closed after all tests
	mu       sync.Mutex // protects default side-effect histories
	wakes    []string   // wake requests intercepted instead of launching the installed agent
	commands [][]string // helper dispatch requests intercepted instead of launching the installed agent
}

// Environment variables that select the roles of the test binary in the syscall guard (linux/amd64).
const (
	guardChildEnv  = "AGENTBUS_TEST_GUARDED"         // set in the traced child, which then runs the suite and does not guard again
	guardProbeEnv  = "AGENTBUS_TEST_GUARD_PROBE"     // set in the self-check child: its value is the directory it writes into
	guardResultEnv = "AGENTBUS_TEST_GUARD_SELFCHECK" // "detected" once the guard's self-check passed; read by TestGuardSelfCheckDetectsClosedWrite
)

// TestMain isolates defaults before any test can load a fixture containing production paths.
func TestMain(m *testing.M) {
	if os.Getenv(guardChildEnv) != "" {
		// Before anything else runs: the tracer must see open calls from the first test on.
		if err := installWriteGuard(); err != nil {
			fmt.Fprintln(os.Stderr, "test isolation: cannot install the syscall guard:", err)
			os.Exit(1)
		}
	}
	if dir := os.Getenv(guardProbeEnv); dir != "" {
		os.Exit(runGuardProbe(dir))
	}
	os.Exit(runIsolatedTests(m))
}

// runGuardProbe is the self-check child of the syscall guard: it writes a file in dir, closes it and exits,
// so the guard must still report the write after the descriptor is gone.
func runGuardProbe(dir string) int {
	file, err := os.OpenFile(filepath.Join(dir, "closed-write.txt"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600) //nolint:gosec // G304: fixed name inside the suite-owned guard directory
	if err != nil {
		fmt.Fprintln(os.Stderr, "guard self-check probe: cannot open file:", err)
		return 2
	}
	if _, err := file.WriteString("probe\n"); err != nil {
		fmt.Fprintln(os.Stderr, "guard self-check probe: cannot write file:", err)
		return 2
	}
	if err := file.Close(); err != nil {
		fmt.Fprintln(os.Stderr, "guard self-check probe: cannot close file:", err)
		return 2
	}
	return 0
}

// runIsolatedTests keeps cleanup outside os.Exit and turns safety-check failures into a failing suite.
func runIsolatedTests(m *testing.M) int {
	originalHomes := []string{os.Getenv("AGENTS_HOME")}
	if home, err := os.UserHomeDir(); err == nil {
		originalHomes = append(originalHomes, filepath.Join(home, "agents"))
	} else {
		fmt.Fprintln(os.Stderr, "test isolation: cannot resolve original home:", err)
		return 1
	}
	if code, guarded := runGuardedTests(originalHomes); guarded {
		return code
	}
	inherited, err := testOpenFiles()
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot inventory inherited descriptors:", err)
		return 1
	}
	root, err := os.MkdirTemp("", "agentbus-suite-")
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot create temporary home:", err)
		return 1
	}
	suiteIsolation.root = root
	code := runTestsInHome(m, root)
	if leaked, err := testHomeDescriptors(originalHomes, inherited); err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot audit open files:", err)
		code = 1
	} else if len(leaked) != 0 {
		fmt.Fprintln(os.Stderr, "test isolation: tests opened files in the real agents home:", leaked)
		code = 1
	}
	if suiteIsolation.log != nil {
		if err := suiteIsolation.log.Close(); err != nil {
			fmt.Fprintln(os.Stderr, "test isolation: cannot close suite log:", err)
			code = 1
		}
	}
	if err := os.RemoveAll(root); err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot remove temporary home:", err)
		code = 1
	}
	return code
}

// runTestsInHome prevents inherited installation configuration from selecting files or a real hub.
func runTestsInHome(m *testing.M, root string) int {
	for key, value := range map[string]string{
		"HOME": root, "AGENTS_HOME": filepath.Join(root, "agents"), "AGENTBUS_DIR": filepath.Join(root, "agents", "bus"),
		"XDG_CONFIG_HOME": filepath.Join(root, "config"), "XDG_CACHE_HOME": filepath.Join(root, "cache"),
	} {
		if err := os.Setenv(key, value); err != nil {
			fmt.Fprintln(os.Stderr, "test isolation: cannot set", key, err)
			return 1
		}
	}
	for _, key := range []string{"LEAD_INSTANCE", "HUB_LISTEN", "HUB_SOCKET", "INSTANCE_NAME", "INSTANCE_CAPABILITIES", "HUB_AUTH", "HUB_TOKEN", "HUB_CERT_SHA256", "HUB_CERT_FINGERPRINT", "HUB_RETRY_SECONDS"} {
		if err := os.Unsetenv(key); err != nil {
			fmt.Fprintln(os.Stderr, "test isolation: cannot clear", key, err)
			return 1
		}
	}
	logDir := filepath.Join(agentsHome(), "logs")
	if err := os.MkdirAll(logDir, 0o700); err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot create log directory:", err)
		return 1
	}
	file, err := os.OpenFile(filepath.Join(logDir, "agentbus.jsonl"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600) //nolint:gosec // G304: fixed log name inside the suite-owned os.MkdirTemp directory
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot open suite log:", err)
		return 1
	}
	suiteIsolation.log = file
	// Do not use setupLogging here: its journal destination belongs to the real host too.
	logger = slog.New(newJSONHandler(file))
	wakeFn = func(_ *Team, task string) {
		suiteIsolation.mu.Lock()
		defer suiteIsolation.mu.Unlock()
		suiteIsolation.wakes = append(suiteIsolation.wakes, task)
	}
	agentCmdFn = func(program, home string, args ...string) {
		suiteIsolation.mu.Lock()
		defer suiteIsolation.mu.Unlock()
		suiteIsolation.commands = append(suiteIsolation.commands, append([]string{program, home}, args...))
	}
	return m.Run()
}

// testHomeDescriptors detects leaked logger/storage handles without mistaking other agents' writes for ours.
func testHomeDescriptors(homes []string, inherited map[string]string) ([]string, error) {
	files, err := testOpenFiles()
	if err != nil {
		return nil, err
	}
	var leaked []string
	for descriptor, path := range files {
		if inherited[descriptor] == path {
			continue // the runner legitimately inherited stdout/stderr files in its own task directory
		}
		for _, home := range homes {
			if home != "" && (path == filepath.Clean(home) || strings.HasPrefix(path, filepath.Clean(home)+string(filepath.Separator))) {
				leaked = append(leaked, path)
				break
			}
		}
	}
	return leaked, nil
}
