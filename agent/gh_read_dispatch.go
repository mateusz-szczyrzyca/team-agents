package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

var (
	// rePullMerge identifies pull request merge endpoints.
	rePullMerge = regexp.MustCompile(`(^|/)pulls/[0-9]+/merge$`)
	// rePull identifies pull request collection and item endpoints.
	rePull = regexp.MustCompile(`(^|/)pulls(/[0-9]+)?$`)
	// reComments identifies issue and pull request comment/review endpoints.
	reComments = regexp.MustCompile(`(^|/)(issues|pulls)/([0-9]+/)?(comments|reviews)(/|$|\?)|(^|/)reviews/[0-9]+/comments(/|$|\?)`)
)

// GHDispatchParams supplies invocation parameters and execution dependencies for ghDispatch.
type GHDispatchParams struct {
	// Args: command arguments (e.g. os.Args[1:]).
	Args []string
	// Env: process environment variables.
	Env []string
	// Stdout: standard output destination.
	Stdout io.Writer
	// Stderr: standard error destination.
	Stderr io.Writer
	// Exec: command executor.
	Exec Executor
}

// ghDispatch evaluates native GitHub read commands and member guards.
func ghDispatch(ctx context.Context, p GHDispatchParams) (int, *GHReadDispatchError) {
	realGH := ghEnvValue(p.Env, "AGENT_REAL_GH")
	if realGH == "" {
		return refuseArg(p, "agent-gh: the real gh is not known (AGENT_REAL_GH)\n", 127, GHReadDispatchMissingRealGH, "the real GitHub executable is missing or not executable")
	}
	for _, a := range p.Args {
		if a == "-t" || a == "--template" || strings.HasPrefix(a, "--template=") {
			return refuseArg(p, "agent-gh: --template is not available to the agents; use --jq\n", 2, GHReadDispatchTemplateRefused, "templates cannot bypass native read filtering")
		}
		if a == "-w" || a == "--web" {
			return refuseArg(p, "agent-gh: --web is not available to the agents\n", 2, GHReadDispatchWebRefused, "web reads cannot enforce trusted-author filtering")
		}
	}
	trustedSet := make(map[string]bool)
	for _, t := range strings.Fields(ghEnvValue(p.Env, "AGENT_GITHUB_TRUSTED_RESOLVED")) {
		trustedSet[t] = true
	}
	if len(p.Args) >= 2 && (p.Args[0] == "pr" || p.Args[0] == "issue") {
		sub, act := p.Args[0], p.Args[1]
		if sub == "pr" && (act == "create" || act == "merge" || act == "ready" || act == "close" || act == "reopen" || act == "edit") {
			if isMem, lead := checkMember(ctx, p); isMem {
				return memberRefusal(p, act, lead)
			}
			if act == "create" {
				if code, err := checkSecondPR(ctx, p, realGH); err != nil {
					return code, err
				}
			}
		}
		switch act {
		case "create", "edit", "comment", "close", "reopen", "merge", "ready", "review":
			return refuseWrite(p)
		case "list":
			return handleList(ctx, p, realGH, trustedSet)
		case "view", "diff", "checkout":
			return handleView(ctx, p, realGH, trustedSet)
		}
	}
	if len(p.Args) > 0 && p.Args[0] == "api" {
		return handleAPI(ctx, p, realGH)
	}
	return runReal(ctx, p, realGH, p.Args, p.Stdout, p.Stderr)
}

// handleAPI intercepts and filters GitHub REST and GraphQL API queries.
func handleAPI(ctx context.Context, p GHDispatchParams, realGH string) (int, *GHReadDispatchError) {
	var jq, path, method string
	var fields bool
	var pass []string
	for i := 1; i < len(p.Args); i++ {
		a := p.Args[i]
		switch {
		case (a == "--jq" || a == "-q") && i+1 < len(p.Args):
			jq, i = p.Args[i+1], i+1
		case (a == "-X" || a == "--method") && i+1 < len(p.Args):
			method, pass, i = p.Args[i+1], append(pass, a, p.Args[i+1]), i+1
		case strings.HasPrefix(a, "--jq="):
			jq = strings.TrimPrefix(a, "--jq=")
		case strings.HasPrefix(a, "-X") || strings.HasPrefix(a, "--method="):
			method, pass = strings.TrimPrefix(strings.TrimPrefix(a, "-X"), "--method="), append(pass, a)
		case strings.HasPrefix(a, "-f") || strings.HasPrefix(a, "-F") || a == "--input":
			fields, pass = true, append(pass, a)
		default:
			if path == "" && !strings.HasPrefix(a, "-") {
				path = a
			}
			pass = append(pass, a)
		}
	}
	if path == "graphql" {
		allPass := strings.Join(pass, " ")
		for _, w := range []string{"comments", "reviews", "reviewThreads", "timelineItems", "body"} {
			if strings.Contains(allPass, w) {
				logGH(p.Env, p.Stderr, EventGitHubGraphQLRefused, "a GraphQL query for comments, reviews or bodies is refused: the agents read them through gh pr view --json or gh api …/comments", nil)
				return refuseArg(p, "agent-gh: a GraphQL query for comments, reviews or bodies is refused — use gh pr view --json comments,reviews or gh api …/comments\n", 1, GHReadDispatchGraphQLRefused, "the GraphQL query requests protected content")
			}
		}
	}
	clean := strings.Split(strings.Split(path, "?")[0], "#")[0]
	isPullMut := rePullMerge.MatchString(clean) || (rePull.MatchString(clean) && (fields || (method != "" && strings.ToUpper(method) != "GET")))
	if isMem, lead := checkMember(ctx, p); isPullMut && isMem {
		logGH(p.Env, p.Stderr, EventGitHubMemberPRRefused, "a team member's gh api PR mutation is refused: its coordinator owns the round's PR", map[string]string{"action": "api", "coordinator": coordOrNone(lead)})
		return refuseArg(p, fmt.Sprintf("agent-gh: a team member does not create or change a PR through gh api — the round's PR is its coordinator's (%s), and the human (or --autopilot) merges it. Push your branch and tell the coordinator what is on it (a board item or a message).\n", coordOrNone(lead)), 1, GHReadDispatchMemberRefused, "only the coordinator may mutate the round pull request")
	}
	if isPullMut || fields || (method != "" && strings.ToUpper(method) != "GET") {
		return refuseWrite(p)
	}
	if reComments.MatchString(path) {
		var out bytes.Buffer
		if code, err := runReal(ctx, p, realGH, append([]string{"api"}, pass...), &out, p.Stderr); err != nil {
			return code, err
		}
		combined, combErr := combinePaginatedJSON(out.Bytes())
		if combErr != nil {
			return 1, combErr
		}
		return filterAndOutput(ctx, p, combined, jq)
	}
	cmdArgs := append([]string{"api"}, pass...)
	if jq != "" {
		cmdArgs = append(cmdArgs, "--jq", jq)
	}
	return runReal(ctx, p, realGH, cmdArgs, p.Stdout, p.Stderr)
}

// combinePaginatedJSON merges concatenated JSON values from gh api --paginate into a single JSON payload.
func combinePaginatedJSON(raw []byte) ([]byte, *GHReadDispatchError) {
	dec, vals := json.NewDecoder(bytes.NewReader(raw)), []any{}
	dec.UseNumber()
	for {
		var v any
		if err := dec.Decode(&v); errors.Is(err, io.EOF) {
			break
		} else if err != nil {
			return nil, opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
		}
		vals = append(vals, v)
	}
	if len(vals) == 0 {
		return raw, nil
	}
	merged, allArr := []any{}, true
	for _, v := range vals {
		if arr, ok := v.([]any); ok && allArr {
			merged = append(merged, arr...)
		} else {
			allArr = false
		}
	}
	var target = vals[0]
	if len(vals) > 1 && allArr {
		target = merged
	}
	return marshalJSON(target)
}

// parseJSONAndJQ separates --json and --jq flags from remaining arguments.
func parseJSONAndJQ(args []string) (jsonf, jq string, rest []string) {
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--json" && i+1 < len(args):
			jsonf, i = args[i+1], i+1
		case (a == "--jq" || a == "-q") && i+1 < len(args):
			jq, i = args[i+1], i+1
		case strings.HasPrefix(a, "--json="):
			jsonf = strings.TrimPrefix(a, "--json=")
		case strings.HasPrefix(a, "--jq="):
			jq = strings.TrimPrefix(a, "--jq=")
		default:
			rest = append(rest, a)
		}
	}
	return
}

// itemAuthor extracts author login from a list or comment item map.
func itemAuthor(it map[string]any) string {
	for _, k := range []string{"author", "user"} {
		if a, ok := it[k].(map[string]any); ok {
			if l, ok := a["login"].(string); ok && l != "" {
				return l
			}
		}
	}
	return ""
}

// str extracts string value from map field.
func str(v any) string {
	s, _ := v.(string)
	return s
}

// decodeJSON decodes JSON data with numeric precision into the target reference.
func decodeJSON(raw []byte, v any) *GHReadDispatchError {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	if err := dec.Decode(v); err != nil {
		return opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
	}
	return nil
}

// marshalJSON encodes value to JSON data and maps serialization failures to typed errors.
func marshalJSON(v any) ([]byte, *GHReadDispatchError) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
	}
	return raw, nil
}

// handleList filters PR/issue lists by trusted authors and non-fork repositories.
func handleList(ctx context.Context, p GHDispatchParams, realGH string, trusted map[string]bool) (int, *GHReadDispatchError) {
	kind := p.Args[0]
	jsonf, jq, rest := parseJSONAndJQ(p.Args[2:])
	want := "number,title,state,url,author"
	if jsonf != "" {
		want = jsonf + ",author"
	}
	if kind == "pr" {
		want += ",isCrossRepository"
		if jsonf == "" {
			want += ",headRefName"
		}
	}
	var out bytes.Buffer
	if code, err := runReal(ctx, p, realGH, append(append([]string{kind, "list"}, rest...), "--json", strings.Join(dedupFields(want), ",")), &out, p.Stderr); err != nil {
		return code, err
	}
	var items []map[string]any
	if err := decodeJSON(out.Bytes(), &items); err != nil {
		return 1, err
	}
	kept := make([]map[string]any, 0)
	for _, it := range items {
		author, num := itemAuthor(it), fmt.Sprintf("%v", it["number"])
		ev, msg := EventGitHubPRIgnored, "there is a pr from a fork and it's ignored"
		if cross, _ := it["isCrossRepository"].(bool); cross {
			logGH(p.Env, p.Stderr, ev, msg, map[string]string{"author": author, "number": num})
			continue
		}
		if !trusted[author] {
			if kind == "issue" {
				ev = EventGitHubIssueIgnored
			}
			logGH(p.Env, p.Stderr, ev, fmt.Sprintf("there is unknown %s from unknown user and it's ignored", kind), map[string]string{"author": author, "number": num})
			continue
		}
		kept = append(kept, it)
	}
	if jsonf != "" {
		raw, err := marshalJSON(kept)
		if err != nil {
			return 1, err
		}
		return outputFilteredJSON(ctx, p, raw, jq)
	}
	var buf strings.Builder
	for _, it := range kept {
		if _, err := fmt.Fprintf(&buf, "#%v\t%s\t%s\t%s\n", it["number"], str(it["title"]), str(it["headRefName"]), str(it["state"])); err != nil {
			return 1, fromStorage(GHReadDispatchStorage, storageErr(err, "format list item"))
		}
	}
	if err := writeOut(p.Stdout, buf.String()); err != nil {
		return 1, fromStorage(GHReadDispatchStorage, err)
	}
	return 0, nil
}

// viewJSON queries metadata via real gh view with requested fields.
func viewJSON(ctx context.Context, p GHDispatchParams, realGH, kind string, pos []string, fields string) ([]byte, int, *GHReadDispatchError) {
	var out bytes.Buffer
	code, err := runReal(ctx, p, realGH, append(append([]string{kind, "view"}, pos...), "--json", fields), &out, p.Stderr)
	return out.Bytes(), code, err
}

// handleView guards view, diff, and checkout on untrusted authors/forks and formats comments.
func handleView(ctx context.Context, p GHDispatchParams, realGH string, trustedSet map[string]bool) (int, *GHReadDispatchError) {
	kind, verb := p.Args[0], p.Args[1]
	jsonf, jq, restArgs := parseJSONAndJQ(p.Args[2:])
	var comments bool
	var pos, rest []string
	for _, a := range restArgs {
		switch {
		case a == "-c" || a == "--comments":
			comments = true
		case strings.HasPrefix(a, "-"):
			rest = append(rest, a)
		default:
			pos = append(pos, a)
		}
	}
	idf := "author"
	if kind != "issue" {
		idf = "author,isCrossRepository"
	}
	rawWho, code, err := viewJSON(ctx, p, realGH, kind, pos, idf)
	if err != nil {
		return code, err
	}
	var who map[string]any
	if decErr := decodeJSON(rawWho, &who); decErr != nil {
		return 1, decErr
	}
	author := itemAuthor(who)
	cross, _ := who["isCrossRepository"].(bool)
	if cross || !trustedSet[author] {
		logGH(p.Env, p.Stderr, EventPRUntrustedIgnored, fmt.Sprintf("a %s from a fork or opened by an unknown account is not shown to the agents", kind), map[string]string{"author": author, "cross_repository": strconv.FormatBool(cross), "ref": strings.Join(pos, " ")})
		return refuseArg(p, fmt.Sprintf("agent-gh: this %s is from a fork or opened by an account that is not trusted — it is not shown\n", kind), 1, GHReadDispatchForkUntrustedRefused, "the target author or repository is untrusted")
	}
	if verb != "view" {
		return runReal(ctx, p, realGH, append(append([]string{kind, verb}, pos...), rest...), p.Stdout, p.Stderr)
	}
	if jsonf != "" {
		raw, code, err := viewJSON(ctx, p, realGH, kind, pos, jsonf)
		if err != nil {
			return code, err
		}
		return filterAndOutput(ctx, p, raw, jq)
	}
	if !comments {
		return runReal(ctx, p, realGH, append([]string{kind, "view"}, pos...), p.Stdout, p.Stderr)
	}
	if code, err := runReal(ctx, p, realGH, append([]string{kind, "view"}, pos...), p.Stdout, p.Stderr); err != nil {
		return code, err
	}
	cf := "comments"
	if kind != "issue" {
		cf = "comments,reviews"
	}
	raw, code, err := viewJSON(ctx, p, realGH, kind, pos, cf)
	if err != nil {
		return code, err
	}
	filtered, ferr := filterJSON(p, raw)
	if ferr != nil {
		return 1, ferr
	}
	var d struct{ Comments, Reviews []map[string]any }
	if decErr := decodeJSON(filtered, &d); decErr != nil {
		return 1, decErr
	}
	var b strings.Builder
	b.WriteString("\n-- comments and reviews (the trusted accounts' only) --\n")
	for _, list := range [][]map[string]any{d.Comments, d.Reviews} {
		for _, e := range list {
			if a, body := itemAuthor(e), str(e["body"]); a != "" && body != "" {
				b.WriteString(a + ": " + body + "\n\n")
			}
		}
	}
	if err := writeOut(p.Stdout, b.String()); err != nil {
		return 1, fromStorage(GHReadDispatchStorage, err)
	}
	return 0, nil
}

// filterJSON applies trusted authors filter and logs ignored entries.
func filterJSON(p GHDispatchParams, raw []byte) ([]byte, *GHReadDispatchError) {
	filtered, ignored, valErr := ghFilterJSON(raw, strings.Fields(ghEnvValue(p.Env, "AGENT_GITHUB_TRUSTED_RESOLVED")), false)
	if valErr != nil {
		return nil, fromValue(GHReadDispatchInvalidJSON, valErr)
	}
	logIgnored(p, ignored)
	return filtered, nil
}

// filterAndOutput filters JSON by trusted authors and formats or queries with jq.
func filterAndOutput(ctx context.Context, p GHDispatchParams, raw []byte, jq string) (int, *GHReadDispatchError) {
	filtered, err := filterJSON(p, raw)
	if err != nil {
		return 1, err
	}
	return outputFilteredJSON(ctx, p, filtered, jq)
}

// logIgnored emits log events for untrusted or marked comments.
func logIgnored(p GHDispatchParams, ignored []ghIgnoredEntry) {
	for _, e := range ignored {
		ev, msg := EventGitHubCommentIgnored, "there is unknown comment from unknown user and it's ignored"
		if e.Reason == ghIgnoreReasonMarkedIgnore {
			ev, msg = EventGitHubCommentMarkedIgnored, "a comment that begins with \"ignore this mate\" is ignored"
		}
		logGH(p.Env, p.Stderr, ev, msg, map[string]string{"author": e.Author, "id": e.ID})
	}
}

// outputFilteredJSON writes filtered JSON or filters with jq if requested.
func outputFilteredJSON(ctx context.Context, p GHDispatchParams, data []byte, jq string) (int, *GHReadDispatchError) {
	if jq == "" {
		if err := writeOut(p.Stdout, string(append(data, '\n'))); err != nil {
			return 1, fromStorage(GHReadDispatchStorage, err)
		}
		return 0, nil
	}
	res, err := p.Exec.Execute(ctx, Command{Name: "jq", Args: []string{"-r", jq}, Env: p.Env, Stdin: bytes.NewReader(data), Stdout: p.Stdout, Stderr: p.Stderr})
	if err != nil || res.ExitCode != 0 {
		code := res.ExitCode
		if err != nil {
			code = 1
		}
		return code, opErr(GHReadDispatchExecFailed, "a GitHub or query subprocess failed")
	}
	return 0, nil
}

// memberRefusal logs and formats member PR mutation refusals.
func memberRefusal(p GHDispatchParams, action, lead string) (int, *GHReadDispatchError) {
	logGH(p.Env, p.Stderr, EventGitHubMemberPRRefused, fmt.Sprintf("a team member's gh pr %s is refused: its coordinator opens the round's PR", action), map[string]string{"action": action, "coordinator": coordOrNone(lead)})
	verb := map[string]string{"create": "open", "ready": "mark ready"}[action]
	if verb == "" {
		verb = action
	}
	detail := fmt.Sprintf("the round's PR is its coordinator's (%s), and the human (or --autopilot) merges it", lead)
	if lead == "" {
		detail = "the round has one, its coordinator's, and your team has no coordinator yet. Push your branch and tell the team what is on it (send_message to \"team\")"
	} else if action == "create" {
		detail = fmt.Sprintf("the round has one, opened by its coordinator (%s). Push your branch and tell the coordinator what is on it (a board item or a message): it merges your work into the round's PR", lead)
	}
	return refuseArg(p, fmt.Sprintf("agent-gh: a team member does not %s a PR — %s.\n", verb, detail), 1, GHReadDispatchMemberRefused, "only the coordinator may mutate the round pull request")
}

// checkSecondPR verifies the task does not already have an open pull request.
func checkSecondPR(ctx context.Context, p GHDispatchParams, realGH string) (int, *GHReadDispatchError) {
	ownPR := ghEnvValue(p.Env, "AGENT_GH_OWN_PR")
	if ownPR == "" {
		return 0, nil
	}
	var out bytes.Buffer
	if _, err := runReal(ctx, p, realGH, []string{"pr", "view", ownPR, "--json", "state"}, &out, io.Discard); err == nil {
		var s map[string]any
		if decodeJSON(out.Bytes(), &s) == nil && s["state"] == "OPEN" {
			logGH(p.Env, p.Stderr, EventGitHubSecondPRRefused, "a second PR of a task whose PR is open is refused", map[string]string{"pr": ownPR})
			return refuseArg(p, fmt.Sprintf("agent-gh: this task's PR is #%s and still open — push to its branch and keep its description up to date; open no other PR.\n", ownPR), 1, GHReadDispatchSecondPRRefused, "the round already has an open pull request")
		}
	}
	return 0, nil
}

// checkMember inspects environment and bus state to determine if caller is a team member.
func checkMember(ctx context.Context, p GHDispatchParams) (bool, string) {
	lead, team, self := ghEnvValue(p.Env, "AGENT_GH_COORDINATOR"), ghEnvValue(p.Env, "AGENT_GH_TEAM"), ghEnvValue(p.Env, "AGENT_GH_SELF")
	if team == "" {
		return lead != "", lead
	}
	bin := ghEnvValue(p.Env, "AGENTBUS_BIN")
	if bin == "" {
		bin = filepath.Join(ghEnvValue(p.Env, "HOME"), ".local", "bin", "agentbus")
	}
	base := ghEnvValue(p.Env, "AGENTS_HOME")
	if base == "" {
		base = filepath.Join(ghEnvValue(p.Env, "HOME"), "agents")
	}
	dir := ghEnvValue(p.Env, "AGENT_GH_BUS_DIR")
	if dir == "" {
		dir = filepath.Join(base, "bus")
	}
	var out, errBuf bytes.Buffer
	res, err := p.Exec.Execute(ctx, Command{Name: bin, Args: []string{"team-get", "--dir", dir, "--team", team}, Env: p.Env, Stdout: &out, Stderr: &errBuf})
	if err != nil || res.ExitCode != 0 {
		code := res.ExitCode
		if code == 0 {
			code = 1
		}
		logGH(p.Env, p.Stderr, EventGitHubCoordinatorUnknown, "the bus did not say who coordinates the team", map[string]string{"team": team, "exit_code": strconv.Itoa(code), "stderr": errBuf.String()})
		return lead != "", lead
	}
	var resp map[string]any
	if decodeJSON(out.Bytes(), &resp) != nil {
		return lead != "", lead
	}
	rlead := str(resp["lead"])
	return rlead == "" || rlead != self, rlead
}

// runReal executes real gh command via the supplied executor.
func runReal(ctx context.Context, p GHDispatchParams, realGH string, args []string, out, err io.Writer) (int, *GHReadDispatchError) {
	res, execErr := p.Exec.Execute(ctx, Command{Name: realGH, Args: args, Env: p.Env, Stdout: out, Stderr: err})
	if execErr == nil && res.ExitCode == 0 {
		return 0, nil
	}
	if (execErr != nil && (errors.Is(execErr, exec.ErrNotFound) || errors.Is(execErr, os.ErrNotExist) || errors.Is(execErr, os.ErrPermission))) || res.ExitCode == 127 {
		if werr := writeOut(p.Stderr, fmt.Sprintf("agent-gh: %s: missing or not executable\n", realGH)); werr != nil {
			return 127, fromStorage(GHReadDispatchStorage, werr)
		}
		return 127, opErr(GHReadDispatchMissingRealGH, "the real GitHub executable is missing or not executable")
	}
	code := res.ExitCode
	if code == 0 {
		code = 1
	}
	return code, opErr(GHReadDispatchExecFailed, "a GitHub or query subprocess failed")
}

// refuseWrite refuses native write requests until R12.
func refuseWrite(p GHDispatchParams) (int, *GHReadDispatchError) {
	return refuseArg(p, "agent-gh: native write formatting is not implemented yet\n", 1, GHReadDispatchWriteUnavailable, "native write formatting is not implemented yet")
}

// refuseArg prints an error message to stderr and returns the typed dispatch error.
func refuseArg(p GHDispatchParams, msg string, code int, cause GHReadDispatchCause, doc string) (int, *GHReadDispatchError) {
	if err := writeOut(p.Stderr, msg); err != nil {
		return code, fromStorage(GHReadDispatchStorage, err)
	}
	return code, opErr(cause, doc)
}

// coordOrNone returns coordinator name or "none".
func coordOrNone(lead string) string {
	if lead == "" {
		return "none"
	}
	return lead
}

// dedupFields returns deduplicated list of comma-separated field names.
func dedupFields(fields string) (res []string) {
	seen := make(map[string]bool)
	for _, f := range strings.Split(fields, ",") {
		if f = strings.TrimSpace(f); f != "" && !seen[f] {
			seen[f], res = true, append(res, f)
		}
	}
	return
}

// writeOut writes string data to destination writer and returns typed storage error on failure.
func writeOut(w io.Writer, s string) *StorageError {
	if _, err := io.WriteString(w, s); err != nil {
		return storageErr(err, "write output")
	}
	return nil
}

// logGH records GitHub read-dispatch events to the append-only warning log.
func logGH(env []string, stderr io.Writer, event LogEvent, msg string, fields map[string]string) {
	if err := ghReadLog(env, event, msg, fields); err != nil {
		if _, writeErr := fmt.Fprintf(stderr, "agent-gh: cannot write the log %s\n", err); writeErr != nil {
			// Failed write to stderr cannot be reported further when stderr itself is broken.
			return
		}
	}
}
