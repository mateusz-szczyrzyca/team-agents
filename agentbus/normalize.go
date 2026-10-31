// normalize reads the JSONL event stream of an agent CLI (Claude Code stream-json,
// Codex exec --json, Antigravity agy stream-json, OpenCode run --format json) and prints a unified
// activity log — one JSON line per event:
//
//	{"ts":"15:04:05","k":"text|cmd|edit|tool|mcp|err|info","t":"..."}
//
// Used by the `agent` script (activity.jsonl) and read by `agent live`.
package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"sort"
	"strings"
)

// activity is one line of a task's activity log (activity.jsonl): a CLI-independent summary of one thing
// the agent did. Keys are short because the file is append-only and grows with every step. `agent live` shows
// these entries and `agent transcript` merges them with the bus messages.
type activity struct {
	TS   Timestamp    `json:"ts"` // local time "2006-01-02 15:04:05"
	Kind ActivityKind `json:"k"`  // what kind of step it was
	Text string       `json:"t"`  // human-readable summary: the message, the command line, "Edit <path>", "reserve {...}", ...
}

// ActivityKind classifies an activity entry. The same values are written by the `agent` script (info, err).
type ActivityKind string

const (
	ActText ActivityKind = "text" // a message of the agent
	ActCmd  ActivityKind = "cmd"  // a shell command
	ActEdit ActivityKind = "edit" // a file change
	ActMCP  ActivityKind = "mcp"  // a call of an agentbus tool
	ActTool ActivityKind = "tool" // any other tool call
	ActErr  ActivityKind = "err"  // an error reported by the CLI or a failed tool
	ActInfo ActivityKind = "info" // run metadata (model, step costs, session)
)

// normalizer turns the JSON event stream of one agent CLI into activity entries.
type normalizer struct {
	enc    *json.Encoder                // stdout: entries truncated to 600 characters (activity.jsonl)
	full   *json.Encoder                // optional: untruncated entries (agent transcript --raw)
	deltas map[float64]*strings.Builder // agy: response text assembled from text_delta chunks, per step
	agyUse agyTokens                    // agy: the tokens of the run's finished steps, one usage line at its result
	err    *StorageError                // the first write error on stdout; stops the normalizer
}

// emit writes one activity entry (truncated) and, if enabled, the full entry. A failure to write stdout is
// recorded in n.err (the pipeline is broken); a failure of the optional full copy is logged and the copy is
// switched off.
func (n *normalizer) emit(kind ActivityKind, text string) {
	text = strings.TrimSpace(text)
	if text == "" || n.err != nil {
		return
	}
	ts := Now()
	if err := n.enc.Encode(activity{TS: ts, Kind: kind, Text: trunc(text, 600)}); err != nil {
		n.err = storageErr(err, "cannot write the activity entry to stdout")
		return
	}
	if n.full != nil {
		if err := n.full.Encode(activity{TS: ts, Kind: kind, Text: text}); err != nil {
			warnSideEffect("write full activity entry", storageErr(err, "cannot write the full activity copy"))
			n.full = nil
		}
	}
}

// str returns the first non-empty string value among keys.
func str(m map[string]any, keys ...string) string {
	for _, k := range keys {
		if v, ok := m[k].(string); ok && v != "" {
			return v
		}
	}
	return ""
}

// The event streams of the CLIs are loosely structured: fields are optional and their types are not
// guaranteed. The accessors below read a missing field, or a field of another type, as the zero value —
// that is the normal case, not an error.

// arr returns a nested array (nil when absent).
func arr(m map[string]any, k string) []any {
	if v, ok := m[k].([]any); ok {
		return v
	}
	return nil
}

// asObj returns v as an object (an empty map when it is something else).
func asObj(v any) map[string]any {
	if m, ok := v.(map[string]any); ok {
		return m
	}
	return map[string]any{}
}

// num returns a numeric field and whether it is present.
func num(m map[string]any, k string) (float64, bool) {
	v, ok := m[k].(float64)
	return v, ok
}

// obj returns a nested object (or an empty map).
func obj(m map[string]any, k string) map[string]any {
	if v, ok := m[k].(map[string]any); ok {
		return v
	}
	return map[string]any{}
}

// compactJSON renders v as compact JSON, truncated to max runes. Values decoded from JSON always encode;
// anything else falls back to Go formatting and is logged.
func compactJSON(v any, maxRunes int) string {
	b, err := json.Marshal(v)
	if err != nil {
		warnOnce("compactJSON", LogUnencodableInput, valueWrap(err, "cannot encode %T", v))
		return trunc(fmt.Sprintf("%v", v), maxRunes)
	}
	return trunc(string(b), maxRunes)
}

// toolSummary returns the kind and a short description of a tool call from its name and parameters.
func toolSummary(name string, input map[string]any) (ActivityKind, string) {
	lname := strings.ToLower(name)
	switch {
	case strings.HasPrefix(lname, "mcp__") || strings.HasPrefix(lname, "mcp_") || strings.HasPrefix(lname, "agentbus_") || strings.HasPrefix(lname, "agentbus."):
		short := name
		if i := strings.LastIndex(name, "__"); i >= 0 {
			short = name[i+2:]
		} else if strings.HasPrefix(lname, "agentbus") {
			short = name[len("agentbus_"):]
		}
		return ActMCP, short + " " + compactJSON(input, 200)
	case lname == "bash" || lname == "execute" || lname == "run_command" || lname == "shell" || strings.Contains(lname, "command"):
		if c := str(input, "command", "CommandLine", "cmd"); c != "" {
			return ActCmd, c
		}
	case strings.Contains(lname, "edit") || strings.Contains(lname, "write") || strings.Contains(lname, "replace"):
		if p := str(input, "file_path", "filePath", "path", "TargetFile", "AbsolutePath", "file"); p != "" {
			return ActEdit, name + " " + p
		}
	}
	if p := str(input, "file_path", "filePath", "path", "pattern", "query", "url", "TargetFile", "AbsolutePath", "Query"); p != "" {
		return ActTool, name + " " + p
	}
	return ActTool, name + " " + compactJSON(input, 160)
}

// claude handles Claude Code stream-json events (system/assistant/user/result).
func (n *normalizer) claude(ev map[string]any) {
	switch ev["type"] {
	case "system":
		if ev["subtype"] == "init" {
			n.emit(ActInfo, "session started, model "+str(ev, "model"))
		}
	case "assistant":
		for _, c := range arr(obj(ev, "message"), "content") {
			item := asObj(c)
			switch item["type"] {
			case "text":
				n.emit(ActText, str(item, "text"))
			case "tool_use":
				k, t := toolSummary(str(item, "name"), obj(item, "input"))
				n.emit(k, t)
			}
		}
	case "user":
		for _, c := range arr(obj(ev, "message"), "content") {
			item := asObj(c)
			if item["type"] == "tool_result" && item["is_error"] == true {
				n.emit(ActErr, fmt.Sprint(item["content"]))
			}
		}
	case "result":
		n.emit(ActInfo, fmt.Sprintf("result: %v, turns: %v", ev["subtype"], ev["num_turns"]))
		if u := obj(ev, "usage"); len(u) > 0 { // the run's tokens: input with what the cache wrote and read, output, cost
			in, _ := num(u, "input_tokens")                     // absent = not reported (0)
			created, _ := num(u, "cache_creation_input_tokens") // absent = not reported (0)
			read, _ := num(u, "cache_read_input_tokens")        // absent = not reported (0)
			out, _ := num(u, "output_tokens")                   // absent = not reported (0)
			cost, _ := num(ev, "total_cost_usd")                // absent = not reported (0)
			n.emit(ActInfo, fmt.Sprintf("usage: in %d (cached %d), out %d, cost $%.4f", int(in+created+read), int(read), int(out), cost))
		}
	}
}

// codex handles `codex exec --json` events (thread/item/turn).
func (n *normalizer) codex(ev map[string]any) {
	typ := str(ev, "type")
	item := obj(ev, "item")
	itype := str(item, "type")
	switch {
	case typ == "thread.started":
		n.emit(ActInfo, "session started "+str(ev, "thread_id"))
	case typ == "item.started" && itype == "command_execution":
		n.emit(ActCmd, str(item, "command"))
	case typ == "item.completed":
		switch itype {
		case "agent_message":
			n.emit(ActText, str(item, "text"))
		case "command_execution":
			if code, ok := num(item, "exit_code"); ok && code != 0 {
				n.emit(ActErr, fmt.Sprintf("exit %d: %s", int(code), str(item, "command")))
			}
		case "file_change":
			var parts []string
			for _, c := range arr(item, "changes") {
				cm := asObj(c)
				parts = append(parts, str(cm, "kind")+" "+str(cm, "path"))
			}
			n.emit(ActEdit, strings.Join(parts, ", "))
		case "mcp_tool_call":
			n.emit(ActMCP, str(item, "tool")+" "+compactJSON(item["arguments"], 200))
		case "web_search":
			n.emit(ActTool, "web_search "+str(item, "query"))
		case "error":
			n.emit(ActErr, str(item, "message"))
		}
	case typ == "turn.completed": // the turn's tokens: input (cached ones included), output
		if u := obj(ev, "usage"); len(u) > 0 {
			in, _ := num(u, "input_tokens")            // absent = not reported (0)
			cached, _ := num(u, "cached_input_tokens") // absent = not reported (0)
			out, _ := num(u, "output_tokens")          // absent = not reported (0)
			n.emit(ActInfo, fmt.Sprintf("usage: in %d (cached %d), out %d", int(in), int(cached), int(out)))
		}
	case typ == "turn.failed":
		n.emit(ActErr, str(obj(ev, "error"), "message"))
	case typ == "error":
		n.emit(ActErr, str(ev, "message"))
	}
}

// agyTokens sums the tokens agy reports for each finished step of a run.
type agyTokens struct {
	in     float64 // input: the new tokens and those the cache read
	cached float64 // the input the cache read
	out    float64 // output: the response and the thinking
	seen   bool    // a step reported its usage
}

// add counts one finished step's usage.
func (u *agyTokens) add(m map[string]any) {
	in, _ := num(m, "input_tokens")          // absent = not reported (0)
	cached, _ := num(m, "cache_read_tokens") // absent = not reported (0)
	out, _ := num(m, "output_tokens")        // absent = not reported (0)
	think, _ := num(m, "thinking_tokens")    // absent = not reported (0)
	u.in += in + cached
	u.cached += cached
	u.out += out + think
	u.seen = true
}

// agy handles Antigravity CLI stream-json events (init/step_update/result).
func (n *normalizer) agy(ev map[string]any) {
	switch ev["event"] {
	case "init":
		n.emit(ActInfo, "session started "+str(ev, "conversation_id"))
	case "step_update":
		su := obj(ev, "step_update")
		idx, _ := num(su, "step_index") // absent = step 0
		if u := obj(su, "usage"); su["state"] == "DONE" && len(u) > 0 {
			n.agyUse.add(u)
		}
		switch su["step_type"] {
		case "agent_response":
			b := n.deltas[idx]
			if b == nil {
				b = &strings.Builder{}
				n.deltas[idx] = b
			}
			b.WriteString(str(su, "text_delta"))
			if su["state"] == "DONE" {
				n.emit(ActText, b.String())
				delete(n.deltas, idx)
			}
		case "tool":
			if su["state"] != "DONE" {
				return
			}
			ti := obj(su, "tool_info")
			k, t := toolSummary(str(su, "tool_name"), obj(ti, "parameters"))
			n.emit(k, t)
			if e := obj(ti, "error"); len(e) > 0 {
				n.emit(ActErr, str(e, "message", "type"))
			}
		}
	case "result":
		r := obj(ev, "result")
		msg := "result: " + str(r, "status")
		if e := str(r, "error"); e != "" {
			n.emit(ActErr, e)
		}
		n.emit(ActInfo, msg)
		if n.agyUse.seen {
			n.emit(ActInfo, fmt.Sprintf("usage: in %d (cached %d), out %d", int(n.agyUse.in), int(n.agyUse.cached), int(n.agyUse.out)))
			n.agyUse = agyTokens{}
		}
	}
}

// opencode: `opencode run --format json` — step_start / text / tool_use / step_finish / error.
func (n *normalizer) opencode(ev map[string]any) {
	part := obj(ev, "part")
	switch ev["type"] {
	case "text":
		n.emit(ActText, str(part, "text"))
	case "tool_use", "tool":
		state := obj(part, "state")
		k, t := toolSummary(str(part, "tool"), obj(state, "input"))
		if title := str(state, "title"); title != "" && k == ActTool {
			t = str(part, "tool") + " " + title
		}
		n.emit(k, t)
		if state["status"] == "error" {
			n.emit(ActErr, str(state, "error"))
		}
	case "step_finish":
		tok := obj(part, "tokens")
		cost, _ := num(part, "cost") // absent = not reported (shown as 0)
		if in, ok := num(tok, "input"); ok {
			out, _ := num(tok, "output") // absent = not reported (shown as 0)
			n.emit(ActInfo, fmt.Sprintf("step: %v, tokens in/out %d/%d, cost $%.4f", part["reason"], int(in), int(out), cost))
		}
	case "error":
		e := obj(ev, "error")
		msg := str(obj(e, "data"), "message")
		if msg == "" {
			msg = str(e, "message", "name")
		}
		if msg == "" {
			msg = compactJSON(ev["error"], 300)
		}
		n.emit(ActErr, msg)
	}
}

// normalize: stdin = raw CLI event stream, stdout = activity JSONL (optionally --full for untruncated copies).
func cmdNormalize(args []string) {
	fs := newFlagSet("normalize")
	typ := fs.String("type", "", "claude | codex | gemini | opencode")
	fullPath := fs.String("full", "", "file for untruncated entries (appended)")
	parseFlags(fs, args)
	handlers := map[string]func(*normalizer, map[string]any){
		"claude": (*normalizer).claude, "codex": (*normalizer).codex, "gemini": (*normalizer).agy, "opencode": (*normalizer).opencode,
	}
	handle, ok := handlers[*typ]
	if !ok {
		exitOn(opErr(CmdNormalizeInvalidType, "--type must be claude, codex, gemini or opencode, got %q", *typ).asError())
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	n := &normalizer{enc: enc, deltas: map[float64]*strings.Builder{}}
	if *fullPath != "" {
		f, err := os.OpenFile(*fullPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
		if err != nil {
			warnSideEffect("open full activity copy", storageErr(err, "cannot open %s", *fullPath))
		} else {
			defer func() {
				if err := f.Close(); err != nil {
					warnSideEffect("close full activity copy", storageErr(err, "cannot close %s", *fullPath))
				}
			}()
			n.full = json.NewEncoder(f)
			n.full.SetEscapeHTML(false)
		}
	}
	r := bufio.NewReaderSize(os.Stdin, 1<<20)
	for lineNo := 1; n.err == nil; lineNo++ {
		line, err := r.ReadBytes('\n')
		if trimmed := strings.TrimSpace(string(line)); trimmed != "" {
			var ev map[string]any
			if jerr := json.Unmarshal(line, &ev); jerr != nil {
				// CLIs occasionally print plain text (warnings, banners) into the event stream
				warnOnce(fmt.Sprintf("normalize:%s:non-json", *typ), LogNonJSONCLILine,
					valueWrap(jerr, "line %d: %s", lineNo, trunc(trimmed, 200)))
			} else {
				handle(n, ev)
			}
		}
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			exitOn(fromStorage(CmdNormalizeReadInput, storageErr(err, "cannot read the CLI event stream")).asError())
		}
	}
	exitOn(fromStorage(CmdNormalizeWriteOutput, n.err).asError())
	// unfinished agy responses (e.g. an interrupted run)
	keys := make([]float64, 0, len(n.deltas))
	for k := range n.deltas {
		keys = append(keys, k)
	}
	sort.Float64s(keys)
	for _, k := range keys {
		n.emit(ActText, n.deltas[k].String())
	}
}
