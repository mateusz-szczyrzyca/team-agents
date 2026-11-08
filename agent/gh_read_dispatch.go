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
	rePullMerge       = regexp.MustCompile(`(^|/)pulls/[0-9]+/merge$`)                                                                      // rePullMerge identifies pull request merge endpoints.
	rePull            = regexp.MustCompile(`(^|/)pulls(/[0-9]+)?$`)                                                                         // rePull identifies pull request collection and item endpoints.
	reComments        = regexp.MustCompile(`(^|/)(issues|pulls)/([0-9]+/)?(comments|reviews)(/|$|\?)|(^|/)reviews/[0-9]+/comments(/|$|\?)`) // reComments identifies issue and pull request comment/review endpoints.
	reGraphQLBlocked  = regexp.MustCompile(`comments|reviews|reviewThreads|timelineItems|body`)                                             // reGraphQLBlocked identifies protected fields in GraphQL requests.
	reGraphQLMutation = regexp.MustCompile(`(?i)\bmutation\b`)                                                                              // reGraphQLMutation identifies mutation operations in GraphQL queries.
	reGraphQLPullMut  = regexp.MustCompile(`(?i)pull_?request`)                                                                             // reGraphQLPullMut identifies pull request operations in GraphQL queries.
)

// GHDispatchParams supplies invocation parameters and execution dependencies for ghDispatch.
type GHDispatchParams struct {
	Args           []string  // Args: command arguments.
	Env            []string  // Env: process environment variables.
	Stdout, Stderr io.Writer // Stdout, Stderr: output destinations.
	Exec           Executor  // Exec: command executor.
}

// ghDispatch evaluates native GitHub read commands and member guards.
func ghDispatch(ctx context.Context, p GHDispatchParams) (int, *GHReadDispatchError) {
	realGH := ghEnvValue(p.Env, "AGENT_REAL_GH")
	if realGH == "" {
		return p.refuse("agent-gh: the real gh is not known (AGENT_REAL_GH)\n", 127, GHReadDispatchMissingRealGH, "the real GitHub executable is missing or not executable")
	}
	trustedSet := make(map[string]bool)
	for _, t := range strings.Fields(ghEnvValue(p.Env, "AGENT_GITHUB_TRUSTED_RESOLVED")) {
		trustedSet[t] = true
	}
	if len(p.Args) > 0 && p.Args[0] == "api" {
		return p.handleAPI(ctx, realGH)
	}
	if len(p.Args) >= 2 && (p.Args[0] == "pr" || p.Args[0] == "issue") {
		sub := p.Args[1]
		switch sub {
		case "list":
			return p.handleList(ctx, realGH, trustedSet)
		case "view", "diff", "checkout":
			return p.handleView(ctx, realGH, trustedSet)
		case "create", "merge", "ready", "close", "reopen", "edit", "comment", "review":
			if p.Args[0] == "pr" && sub != "comment" && sub != "review" {
				if isMem, lead := p.checkMember(ctx); isMem {
					return p.memberRefusal(sub, lead)
				}
				if own := ghEnvValue(p.Env, "AGENT_GH_OWN_PR"); sub == "create" && own != "" {
					var s map[string]any
					if _, err := p.runJSON(ctx, realGH, []string{"pr", "view", own, "--json", "state"}, &s); err == nil && s["state"] == "OPEN" {
						p.log(EventGitHubSecondPRRefused, "a second PR of a task whose PR is open is refused", map[string]string{"pr": own})
						return p.refuse("agent-gh: this task's PR is #"+own+" and still open — push to its branch and keep its description up to date; open no other PR.\n", 1, GHReadDispatchSecondPRRefused, "the round already has an open pull request")
					}
				}
			}
			return p.refuse("agent-gh: native write formatting is not implemented yet\n", 1, GHReadDispatchWriteUnavailable, "native write formatting is not implemented yet")
		}
	}
	return p.run(ctx, realGH, p.Args, nil, p.Stdout, p.Stderr)
}

// handleAPI intercepts and filters GitHub REST and GraphQL API queries.
func (p GHDispatchParams) handleAPI(ctx context.Context, realGH string) (int, *GHReadDispatchError) {
	var pass []string
	var jq, path, method string
	var fields, hasInput bool
	rawArgs := p.Args[1:]
	for i := 0; i < len(rawArgs); i++ {
		a := rawArgs[i]
		switch {
		case (a == "--jq" || a == "-q") && i+1 < len(rawArgs):
			jq, i = rawArgs[i+1], i+1
		case strings.HasPrefix(a, "--jq="):
			jq = a[5:]
		case a == "-t" || a == "--template" || strings.HasPrefix(a, "--template="):
			return p.refuse("agent-gh: --template is not available to the agents; use --jq\n", 2, GHReadDispatchTemplateRefused, "templates cannot bypass native read filtering")
		case (a == "-X" || a == "--method") && i+1 < len(rawArgs):
			method = rawArgs[i+1]
			pass = append(pass, a, rawArgs[i+1])
			i++
		case strings.HasPrefix(a, "--method="):
			method = a[9:]
			pass = append(pass, a)
		case strings.HasPrefix(a, "-X="):
			method = a[3:]
			pass = append(pass, a)
		case strings.HasPrefix(a, "-X") && len(a) > 2:
			method = a[2:]
			pass = append(pass, a)
		case (a == "-f" || a == "--raw-field" || a == "-F" || a == "--field" || a == "--input") && i+1 < len(rawArgs):
			fields, hasInput = true, hasInput || a == "--input" || ((a == "-F" || a == "--field") && isTypedQueryFile(rawArgs[i+1]))
			pass = append(pass, a, rawArgs[i+1])
			i++
		case strings.HasPrefix(a, "--input="):
			fields, hasInput = true, true
			pass = append(pass, a)
		case strings.HasPrefix(a, "--field=") || strings.HasPrefix(a, "--raw-field=") ||
			strings.HasPrefix(a, "-f=") || strings.HasPrefix(a, "-F=") ||
			(strings.HasPrefix(a, "-f") && len(a) > 2) || (strings.HasPrefix(a, "-F") && len(a) > 2):
			fields, hasInput = true, hasInput || isTypedFieldArg(a)
			pass = append(pass, a)
		case (a == "-H" || a == "--header" || a == "--hostname" || a == "--cache" || a == "-c" || a == "--preview") && i+1 < len(rawArgs):
			pass = append(pass, a, rawArgs[i+1])
			i++
		default:
			if path == "" && !strings.HasPrefix(a, "-") {
				path = a
			}
			pass = append(pass, a)
		}
	}
	pass = append([]string{"api"}, pass...)
	clean := strings.Split(strings.Split(path, "?")[0], "#")[0]
	isPullMut := false
	isMut := (method != "" && !strings.EqualFold(method, "GET")) || (method == "" && fields && path != "graphql")
	if path == "graphql" {
		passStr := strings.Join(pass, " ")
		if reGraphQLBlocked.MatchString(passStr) {
			p.log(EventGitHubGraphQLRefused, "a GraphQL query for comments, reviews or bodies is refused: the agents read them through gh pr view --json or gh api …/comments", nil)
			return p.refuse("agent-gh: a GraphQL query for comments, reviews or bodies is refused — use gh pr view --json comments,reviews or gh api …/comments\n", 1, GHReadDispatchGraphQLRefused, "the GraphQL query requests protected content")
		}
		if hasInput || reGraphQLMutation.MatchString(passStr) {
			isMut, isPullMut = true, reGraphQLPullMut.MatchString(passStr)
		}
	} else if rePullMerge.MatchString(clean) {
		isPullMut = true
	} else if rePull.MatchString(clean) {
		if method != "" {
			isPullMut = !strings.EqualFold(method, "GET")
		} else if fields {
			isPullMut = true
		}
	}
	if isMem, lead := p.checkMember(ctx); isPullMut && isMem {
		return p.memberRefusal("api", lead)
	}
	if isPullMut || isMut {
		return p.refuse("agent-gh: native write formatting is not implemented yet\n", 1, GHReadDispatchWriteUnavailable, "native write formatting is not implemented yet")
	}
	if reComments.MatchString(path) {
		raw, code, err := p.runBuf(ctx, realGH, pass)
		if err != nil {
			return code, err
		}
		dec, merged, pages, v := json.NewDecoder(bytes.NewReader(raw)), []any{}, 0, any(nil)
		dec.UseNumber() // Preserve numeric precision for large IDs
		var firstPage any
		var decErr error
		var allArrays = true // Track if all pages are arrays
		for decErr = dec.Decode(&v); decErr == nil; decErr = dec.Decode(&v) {
			pages++
			if pages == 1 {
				firstPage = v
			}
			if arr, ok := v.([]any); ok {
				merged = append(merged, arr...)
			} else {
				allArrays = false // Non-array page found
			}
		}
		// Non-EOF decode error means malformed JSON
		if decErr != nil && decErr != io.EOF {
			return p.refuse("", 1, GHReadDispatchInvalidJSON, "a paginated GitHub response contains malformed JSON")
		}
		if pages == 0 {
			return p.refuse("", 1, GHReadDispatchInvalidJSON, "a GitHub response is empty")
		}
		// Multiple pages: only merge arrays if ALL pages are arrays, otherwise use first page
		if pages > 1 {
			if allArrays && len(merged) > 0 {
				return p.outputJSON(ctx, merged, jq, true)
			}
			// Any non-array page found, return first page
			return p.outputJSON(ctx, firstPage, jq, true)
		}
		// Single page: return as-is (preserves original numeric format)
		return p.outputJSON(ctx, raw, jq, true)
	}
	if jq != "" {
		pass = append(pass, "--jq", jq)
	}
	return p.run(ctx, realGH, pass, nil, p.Stdout, p.Stderr)
}

// checkReadFlags refuses output modes that bypass filtering only on read commands.
func (p GHDispatchParams) checkReadFlags(args []string) (int, *GHReadDispatchError) {
	for _, a := range args {
		if a == "-t" || a == "--template" || strings.HasPrefix(a, "--template=") {
			return p.refuse("agent-gh: --template is not available to the agents; use --jq\n", 2, GHReadDispatchTemplateRefused, "templates cannot bypass native read filtering")
		} else if a == "-w" || a == "--web" {
			return p.refuse("agent-gh: --web is not available to the agents\n", 2, GHReadDispatchWebRefused, "web reads cannot enforce trusted-author filtering")
		}
	}
	return 0, nil
}

// parseFlags extracts --json, --jq, and -c/--comments flags from arguments.
func parseFlags(args []string) (jsonf, jq string, comments bool, rest []string) {
	for i := 0; i < len(args); i++ {
		switch a := args[i]; {
		case a == "--json" && i+1 < len(args):
			jsonf, i = args[i+1], i+1
		case (a == "--jq" || a == "-q") && i+1 < len(args):
			jq, i = args[i+1], i+1
		case strings.HasPrefix(a, "--json="):
			jsonf = a[7:]
		case strings.HasPrefix(a, "--jq="):
			jq = a[5:]
		case a == "-c" || a == "--comments":
			comments = true
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
			return str(a["login"])
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
	if dec.Decode(v) != nil {
		return opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
	}
	return nil
}

// handleList filters PR/issue lists by trusted authors and non-fork repositories.
func (p GHDispatchParams) handleList(ctx context.Context, realGH string, trusted map[string]bool) (int, *GHReadDispatchError) {
	if code, err := p.checkReadFlags(p.Args[2:]); err != nil {
		return code, err
	}
	kind := p.Args[0]
	jsonf, jq, _, rest := parseFlags(p.Args[2:])
	flds := "number,title,state,url,author"
	if jsonf != "" {
		flds = jsonf + ",author"
	} else if kind == "pr" {
		flds += ",headRefName"
	}
	if kind == "pr" {
		flds += ",isCrossRepository"
	}
	var items []map[string]any
	if code, err := p.runJSON(ctx, realGH, append(append([]string{kind, "list"}, rest...), "--json", flds), &items); err != nil {
		return code, err
	}
	kept := make([]map[string]any, 0, len(items))
	for _, it := range items {
		author, num := itemAuthor(it), fmt.Sprintf("%v", it["number"])
		if it["isCrossRepository"] == true {
			p.log(EventGitHubPRIgnored, "there is a pr from a fork and it's ignored", map[string]string{"author": author, "number": num})
		} else if !trusted[author] {
			ev := EventGitHubPRIgnored
			if kind == "issue" {
				ev = EventGitHubIssueIgnored
			}
			p.log(ev, "there is unknown "+kind+" from unknown user and it's ignored", map[string]string{"author": author, "number": num})
		} else {
			kept = append(kept, it)
		}
	}
	if jsonf != "" {
		return p.outputJSON(ctx, kept, jq, false)
	}
	var out string
	for _, it := range kept {
		out += fmt.Sprintf("#%v\t%s\t%s\t%s\n", it["number"], str(it["title"]), str(it["headRefName"]), str(it["state"]))
	}
	return writeResult(p.Stdout, out)
}

// handleView guards view, diff, and checkout on untrusted authors/forks and formats comments.
func (p GHDispatchParams) handleView(ctx context.Context, realGH string, trustedSet map[string]bool) (int, *GHReadDispatchError) {
	if code, err := p.checkReadFlags(p.Args[2:]); err != nil {
		return code, err
	}
	kind, verb := p.Args[0], p.Args[1]
	jsonf, jq, comments, pass := parseFlags(p.Args[2:])
	idf, cf := "author", "comments"
	if kind != "issue" {
		idf, cf = "author,isCrossRepository", "comments,reviews"
	}
	vargs := append([]string{kind, "view"}, pass...)
	var who map[string]any
	if code, err := p.runJSON(ctx, realGH, append(vargs, "--json", idf), &who); err != nil {
		return code, err
	}
	author, cross := itemAuthor(who), who["isCrossRepository"] == true
	if cross || !trustedSet[author] {
		p.log(EventPRUntrustedIgnored, "a "+kind+" from a fork or opened by an unknown account is not shown to the agents", map[string]string{"author": author, "cross_repository": strconv.FormatBool(cross), "ref": strings.Join(pass, " ")})
		return p.refuse("agent-gh: this "+kind+" is from a fork or opened by an account that is not trusted — it is not shown\n", 1, GHReadDispatchForkUntrustedRefused, "the target author or repository is untrusted")
	}
	if verb != "view" {
		return p.run(ctx, realGH, append([]string{kind, verb}, pass...), nil, p.Stdout, p.Stderr)
	}
	fld := jsonf
	if fld == "" {
		if code, err := p.run(ctx, realGH, vargs, nil, p.Stdout, p.Stderr); err != nil || !comments {
			return code, err
		}
		fld = cf
	}
	raw, code, err := p.runBuf(ctx, realGH, append(vargs, "--json", fld))
	if err != nil {
		return code, err
	}
	if jsonf != "" {
		return p.outputJSON(ctx, raw, jq, true)
	}
	var d struct{ Comments, Reviews []map[string]any }
	if raw, err = p.filterJSON(raw); err != nil {
		return 1, err
	} else if err = decodeJSON(raw, &d); err != nil {
		return 1, err
	}
	out := "\n-- comments and reviews (the trusted accounts' only) --\n"
	for _, e := range append(d.Comments, d.Reviews...) {
		if a, body := itemAuthor(e), str(e["body"]); a != "" && body != "" {
			out += a + ": " + body + "\n\n"
		}
	}
	return writeResult(p.Stdout, out)
}

// filterJSON applies trusted authors filter and logs ignored entries.
func (p GHDispatchParams) filterJSON(raw []byte) ([]byte, *GHReadDispatchError) {
	filtered, ignored, valErr := ghFilterJSON(raw, strings.Fields(ghEnvValue(p.Env, "AGENT_GITHUB_TRUSTED_RESOLVED")), false)
	if valErr != nil {
		return nil, fromValue(GHReadDispatchInvalidJSON, valErr)
	}
	for _, e := range ignored {
		ev, msg := EventGitHubCommentIgnored, "there is unknown comment from unknown user and it's ignored"
		if e.Reason == ghIgnoreReasonMarkedIgnore {
			ev, msg = EventGitHubCommentMarkedIgnored, "a comment that begins with \"ignore this mate\" is ignored"
		}
		p.log(ev, msg, map[string]string{"author": e.Author, "id": e.ID})
	}
	return filtered, nil
}

// outputJSON writes formatted JSON or queries with jq if requested.
func (p GHDispatchParams) outputJSON(ctx context.Context, data any, jq string, filter bool) (int, *GHReadDispatchError) {
	raw, ok := data.([]byte)
	if !ok {
		var err error
		if raw, err = json.Marshal(data); err != nil {
			return 1, opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
		}
	}
	if filter {
		var err *GHReadDispatchError
		if raw, err = p.filterJSON(raw); err != nil {
			return 1, err
		}
	}
	if jq == "" {
		return writeResult(p.Stdout, string(raw)+"\n")
	}
	return p.run(ctx, "jq", []string{"-r", jq}, bytes.NewReader(raw), p.Stdout, p.Stderr)
}

// memberRefusal logs and formats member PR mutation refusals.
func (p GHDispatchParams) memberRefusal(action, lead string) (int, *GHReadDispatchError) {
	coord := lead
	if coord == "" {
		coord = "none"
	}
	if action == "api" {
		p.log(EventGitHubMemberPRRefused, "a team member's gh api PR mutation is refused: its coordinator owns the round's PR", map[string]string{"action": action, "coordinator": coord})
		return p.refuse("agent-gh: a team member does not create or change a PR through gh api — the round's PR is its coordinator's ("+coord+"), and the human (or --autopilot) merges it. Push your branch and tell the coordinator what is on it (a board item or a message).\n", 1, GHReadDispatchMemberRefused, "only the coordinator may mutate the round pull request")
	}
	p.log(EventGitHubMemberPRRefused, "a team member's gh pr "+action+" is refused: its coordinator opens the round's PR", map[string]string{"action": action, "coordinator": coord})
	verb, detail := action, "the round's PR is its coordinator's ("+lead+"), and the human (or --autopilot) merges it"
	switch action {
	case "create":
		verb, detail = "open", "the round has one, opened by its coordinator ("+lead+"). Push your branch and tell the coordinator what is on it (a board item or a message): it merges your work into the round's PR"
	case "ready":
		verb = "mark ready"
	}
	if lead == "" {
		detail = "the round has one, its coordinator's, and your team has no coordinator yet. Push your branch and tell the team what is on it (send_message to \"team\")"
	}
	return p.refuse("agent-gh: a team member does not "+verb+" a PR — "+detail+".\n", 1, GHReadDispatchMemberRefused, "only the coordinator may mutate the round pull request")
}

// checkMember inspects environment and bus state to determine if caller is a team member.
func (p GHDispatchParams) checkMember(ctx context.Context) (bool, string) {
	lead, team, self := ghEnvValue(p.Env, "AGENT_GH_COORDINATOR"), ghEnvValue(p.Env, "AGENT_GH_TEAM"), ghEnvValue(p.Env, "AGENT_GH_SELF")
	if team == "" {
		return lead != "", lead
	}
	bin, dir := ghEnvValue(p.Env, "AGENTBUS_BIN"), ghEnvValue(p.Env, "AGENT_GH_BUS_DIR")
	if bin == "" {
		bin = filepath.Join(ghEnvValue(p.Env, "HOME"), ".local/bin/agentbus")
	}
	if dir == "" {
		dir = filepath.Join(ghEnvValue(p.Env, "HOME"), "agents/bus")
		if base := ghEnvValue(p.Env, "AGENTS_HOME"); base != "" {
			dir = filepath.Join(base, "bus")
		}
	}
	var out, errOut bytes.Buffer
	var resp map[string]any
	res, err := p.Exec.Execute(ctx, Command{Name: bin, Args: []string{"team-get", "--dir", dir, "--team", team}, Env: p.Env, Stdout: &out, Stderr: &errOut})
	if err != nil || res.ExitCode != 0 || decodeJSON(out.Bytes(), &resp) != nil {
		p.log(EventGitHubCoordinatorUnknown, "the bus did not say who coordinates the team", map[string]string{"team": team, "exit_code": strconv.Itoa(max(res.ExitCode, 1)), "stderr": errOut.String()[:min(errOut.Len(), 500)]})
		return lead != "", lead
	}
	lead = str(resp["lead"])
	return lead == "" || lead != self, lead
}

// run executes a command via the supplied executor and converts failures to dispatch errors.
func (p GHDispatchParams) run(ctx context.Context, name string, args []string, in io.Reader, out, err io.Writer) (int, *GHReadDispatchError) {
	res, execErr := p.Exec.Execute(ctx, Command{Name: name, Args: args, Env: p.Env, Stdin: in, Stdout: out, Stderr: err})
	if execErr == nil && res.ExitCode == 0 {
		return 0, nil
	}
	if (execErr != nil && (errors.Is(execErr, exec.ErrNotFound) || errors.Is(execErr, os.ErrNotExist) || errors.Is(execErr, os.ErrPermission))) || res.ExitCode == 127 {
		return p.refuse("agent-gh: "+name+": missing or not executable\n", 127, GHReadDispatchMissingRealGH, "the real GitHub executable is missing or not executable")
	}
	if execErr != nil {
		return p.refuse("agent-gh: "+name+": a GitHub or query subprocess failed\n", max(res.ExitCode, 1), GHReadDispatchExecFailed, "a GitHub or query subprocess failed")
	}
	return max(res.ExitCode, 1), opErr(GHReadDispatchExecFailed, "a GitHub or query subprocess failed")
}

// runBuf executes real gh and captures stdout into a byte slice.
func (p GHDispatchParams) runBuf(ctx context.Context, realGH string, args []string) ([]byte, int, *GHReadDispatchError) {
	var out bytes.Buffer
	code, err := p.run(ctx, realGH, args, nil, &out, p.Stderr)
	return out.Bytes(), code, err
}

// runJSON queries real gh and decodes JSON output into target.
func (p GHDispatchParams) runJSON(ctx context.Context, realGH string, args []string, target any) (int, *GHReadDispatchError) {
	raw, code, err := p.runBuf(ctx, realGH, args)
	if err == nil && decodeJSON(raw, target) != nil {
		return 1, opErr(GHReadDispatchInvalidJSON, "a filtered GitHub response is malformed")
	}
	return code, err
}

// refuse prints an error message to stderr and returns the typed dispatch error.
func (p GHDispatchParams) refuse(msg string, code int, cause GHReadDispatchCause, doc string) (int, *GHReadDispatchError) {
	if err := writeOut(p.Stderr, msg); err != nil {
		return code, fromStorage(GHReadDispatchStorage, err)
	}
	return code, opErr(cause, doc)
}

// writeOut writes string data to destination writer and returns typed storage error on failure or short write.
func writeOut(w io.Writer, s string) *StorageError {
	if n, err := io.WriteString(w, s); err != nil || n < len(s) {
		if err == nil {
			err = io.ErrShortWrite
		}
		return storageErr(err, "write output")
	}
	return nil
}

// writeResult writes string data to destination writer and returns exit code with dispatch error.
func writeResult(w io.Writer, s string) (int, *GHReadDispatchError) {
	if err := writeOut(w, s); err != nil {
		return 1, fromStorage(GHReadDispatchStorage, err)
	}
	return 0, nil
}

// log records GitHub read-dispatch events to the append-only warning log.
func (p GHDispatchParams) log(event LogEvent, msg string, fields map[string]string) {
	if err := ghReadLog(p.Env, event, msg, fields); err != nil {
		if _, writeErr := fmt.Fprintf(p.Stderr, "agent-gh: cannot write the log %s\n", err); writeErr != nil {
			return
		}
	}
}

// isTypedQueryFile reports whether parameter val sets query to a file or stdin in a typed field flag.
func isTypedQueryFile(val string) bool {
	k, v, ok := strings.Cut(val, "=")
	if !ok {
		return false
	}
	return strings.TrimSpace(k) == "query" && strings.HasPrefix(strings.TrimSpace(v), "@")
}

// isTypedFieldArg reports whether flag arg is a typed field flag loading a GraphQL query from a file or stdin.
func isTypedFieldArg(arg string) bool {
	switch {
	case strings.HasPrefix(arg, "--field="):
		return isTypedQueryFile(arg[len("--field="):])
	case strings.HasPrefix(arg, "-F="):
		return isTypedQueryFile(arg[len("-F="):])
	case strings.HasPrefix(arg, "-F") && len(arg) > 2:
		return isTypedQueryFile(arg[2:])
	default:
		return false
	}
}
