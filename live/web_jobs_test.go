package main

import (
	"errors"
	"flag"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestWebJobsAndInstancesViews replaces the round-nine placeholder with actual HTTP view coverage.
func TestWebJobsAndInstancesViews(t *testing.T) {
	home := fixture(t)
	now := time.Now().UTC()
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Jobs":[
 {"Request":{"ID":"remote-1","Repository":"demo","Selector":"normal","Requires":["hw:rp2350"]},"State":"waiting","Node":"node-a","TaskID":"T remote","Branch":"agents/result","PRURL":"https://example.com/pr/1","Result":"build <passed>","History":[{"State":"waiting","At":%q}]},
 {"Request":{"ID":"remote-2","Selector":"worker"},"State":"running","Node":"node-old"}],
 "Nodes":{"node-a":{"Heartbeat":{"Node":"node-a","Capabilities":["os:linux","hw:rp2350"],"FreeAgents":[{"Name":"worker-1","Tier":"worker"}],"WebURL":"https://example.com/node/"},"LastSeen":%q},
 "node-old":{"Heartbeat":{"Node":"node-old","Capabilities":["arch:arm64"]},"LastSeen":%q}},"FutureField":true}`, now.Format(time.RFC3339Nano), now.Format(time.RFC3339Nano), now.Add(-time.Hour).Format(time.RFC3339Nano)))
	_, ts := testServer(t, home)
	for _, tc := range []struct {
		path string   // route to render
		want []string // public job or node fields
	}{
		{"/jobs", []string{"remote-1", "waiting", "node-a", "normal", "agents/result", `href="https://example.com/pr/1"`, "build &lt;passed&gt;", `href="https://example.com/node/task/T%20remote"`, "remote-2", "node offline"}},
		{"/instances", []string{"node-a", "hw:rp2350", "worker-1", "worker", "online", "node-old", "arch:arm64", "offline", now.Format(time.RFC3339Nano)}},
	} {
		code, body := get(t, ts, tc.path)
		if code != http.StatusOK {
			t.Fatalf("%s: status %d, body %.200s", tc.path, code, body)
		}
		mustContain(t, tc.path, body, tc.want...)
		mustContain(t, tc.path, body, `href="/jobs"`, `href="/instances"`)
	}
}

// TestWebJobsStorageErrors distinguishes an absent registry from corrupt or unreadable storage.
func TestWebJobsStorageErrors(t *testing.T) {
	for _, tc := range []struct {
		name     string // storage condition
		contents string // empty means leave the file absent
		want     int    // HTTP result
	}{{"absent", "", 200}, {"directory", "", 500}, {"empty", " ", 500}, {"malformed", "{", 500}, {"null", "null", 500}, {"future", `{"Version":2}`, 500}, {"bad timestamp", `{"Version":1,"Nodes":{"n":{"LastSeen":"yesterday"}}}`, 500}} {
		t.Run(tc.name, func(t *testing.T) {
			home := fixture(t)
			if tc.name == "directory" {
				writeFile(t, filepath.Join(home, "bus", "jobs.json", "child"), "not a registry")
			} else if tc.contents != "" {
				writeFile(t, filepath.Join(home, "bus", "jobs.json"), tc.contents)
			}
			_, ts := testServer(t, home)
			for _, path := range []string{"/jobs", "/instances"} {
				code, body := get(t, ts, path)
				if code != tc.want {
					t.Fatalf("%s: status %d want %d", path, code, tc.want)
				}
				if code == 500 {
					mustContain(t, path, body, "Storage", "jobs.json")
					_, readErr := readWebJobs(filepath.Join(home, "bus"))
					var typed *OpError[ViewCause]
					if readErr == nil || !errors.As(readErr, &typed) || typed.Cause != ViewStorage {
						t.Fatalf("expected typed storage cause, got %v", readErr)
					}
				}
			}
		})
	}
}

// TestWebJobsUnsafeLinks keeps network-provided URLs and result text inert in the browser.
func TestWebJobsUnsafeLinks(t *testing.T) {
	home := fixture(t)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), `{"Version":1,"Jobs":[{"Request":{"ID":"x","Selector":"worker"},"Node":"n","PRURL":"javascript:alert(1)","Result":"<script>alert(2)</script>"}],"Nodes":{"n":{"Heartbeat":{"Node":"n","WebURL":"javascript:alert(3)"}}}}`)
	_, ts := testServer(t, home)
	for _, path := range []string{"/jobs", "/instances"} {
		code, body := get(t, ts, path)
		if code != 200 {
			t.Fatalf("%s status %d", path, code)
		}
		if strings.Contains(body, `href="javascript:`) || strings.Contains(body, "<script>alert") || strings.Contains(body, "ZgotmplZ") {
			t.Fatalf("unsafe or broken link in %s", path)
		}
	}
}

// TestWebNodeTTLFlag parses the default, configured, and invalid command-line heartbeat timeouts.
func TestWebNodeTTLFlag(t *testing.T) {
	for _, tc := range []struct {
		raw     string // explicit flag value, empty for the backwards-compatible default
		seconds int    // expected positive seconds, or zero for an invalid argument
	}{{"", 90}, {"37", 37}, {"0", 0}, {"-1", 0}, {"oops", 0}, {"9223372036854775807", 0}} {
		fs := flag.NewFlagSet("web", flag.ContinueOnError)
		value := webNodeTTLFlag(fs)
		args := []string{}
		if tc.raw != "" {
			args = append(args, "--node-ttl", tc.raw)
		}
		if err := fs.Parse(args); err != nil {
			t.Fatal(err)
		}
		ttl, err := parseWebNodeTTL(*value)
		if tc.seconds == 0 {
			var typed *OpError[NodeTTLCause]
			if err == nil || !errors.As(err, &typed) || typed.Cause != NodeTTLInvalid {
				t.Fatalf("%q: expected typed invalid timeout, got %v", tc.raw, err)
			}
		} else if err != nil || ttl != time.Duration(tc.seconds)*time.Second {
			t.Fatalf("%q: timeout %v error %v", tc.raw, ttl, err)
		}
	}
}

// TestWebJobsConfiguredTTL honors the lead's configured deadline rather than assuming ninety seconds.
func TestWebJobsConfiguredTTL(t *testing.T) {
	home := fixture(t)
	seen := time.Now().UTC().Add(-time.Hour).Format(time.RFC3339Nano)
	writeFile(t, filepath.Join(home, "bus", "jobs.json"), fmt.Sprintf(`{"Version":1,"Nodes":{"recent":{"Heartbeat":{"Node":"recent"},"LastSeen":%q}}}`, seen))
	server, ts := testServer(t, home)
	server.cfg.nodeTTL = 2 * time.Hour
	code, body := get(t, ts, "/instances")
	if code != 200 {
		t.Fatalf("status %d", code)
	}
	mustContain(t, "custom deadline", body, "online")
	if strings.Contains(body, ">offline<") {
		t.Fatal("custom deadline ignored")
	}
}
