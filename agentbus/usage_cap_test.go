package main

import (
	"regexp"
	"strconv"
	"strings"
	"testing"
)

// A message longer than messageTextMax reaches its readers cut, with how to read it whole (read_message): the bus keeps
// it whole. A short one is as it was; an unknown message is not found.
func TestALongMessageIsCutAndReadWhole(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "w": {"normal", "b", "active"}})
	lead, w := server(b, "lead", inTeam("T")), server(b, "w", inTeam("T"))
	long := "AUDIT START " + strings.Repeat("a checked line. ", 400) + "AUDIT END"
	chk(t).ok(toolResult(lead.send("w", long)))
	chk(t).ok(toolResult(lead.send("w", "a short one")))
	out := chk(t).ok(toolResult(w.read(0)))
	if !strings.Contains(out, "AUDIT START") || strings.Contains(out, "AUDIT END") || !strings.Contains(out, "a short one") {
		t.Fatalf("the long message is not cut, or the short one is missing:\n%s", out)
	}
	m := regexp.MustCompile(`read_message id=(\d+)`).FindStringSubmatch(out)
	if m == nil {
		t.Fatalf("no way to read the rest:\n%s", out)
	}
	id, err := strconv.Atoi(m[1])
	if err != nil {
		t.Fatal(err)
	}
	if full := chk(t).ok(toolResult(w.readMessage(id))); !strings.Contains(full, "AUDIT END") {
		t.Errorf("read_message gives the message whole, got %d characters", len(full))
	}
	failsWith(t, ReadMessageNoSuchMessage)(w.readMessage(99999))
}

// Each CLI's report of the tokens a run used becomes the same activity line: "usage: in N (cached C), out M, cost $X".
func TestEachCLIsTokenUsageIsOneActivityLine(t *testing.T) {
	has := func(acts []activity, want string) bool {
		for _, a := range acts {
			if a.Text == want {
				return true
			}
		}
		return false
	}
	cl := normalizeLines(t, "claude", `{"type":"result","subtype":"success","num_turns":3,"total_cost_usd":0.042,`+
		`"usage":{"input_tokens":1200,"cache_creation_input_tokens":800,"cache_read_input_tokens":9000,"output_tokens":300}}`)
	if want := "usage: in 11000 (cached 9000), out 300, cost $0.0420"; !has(cl, want) {
		t.Errorf("claude: no %q in %v", want, cl)
	}
	cx := normalizeLines(t, "codex", `{"type":"turn.completed","usage":{"input_tokens":5000,"cached_input_tokens":4000,"output_tokens":200}}`)
	if want := "usage: in 5000 (cached 4000), out 200"; !has(cx, want) {
		t.Errorf("codex: no %q in %v", want, cx)
	}
}
