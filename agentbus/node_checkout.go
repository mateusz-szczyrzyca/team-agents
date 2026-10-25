package main

import (
	"context"
	"crypto/rand"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// NodeCheckoutRequest contains job data; repository paths come exclusively from local configuration.
type NodeCheckoutRequest struct {
	Repository string // logical allow-listed repository name
	BaseRef    string // validated ref to fetch when no exact commit is required
	BaseSHA    string // optional exact commit required by the requester
}

// NodeCheckoutResult identifies an immutable base for the shared task-preparation path.
type NodeCheckoutResult struct {
	Root   string `json:"root"`   // locally configured repository path
	Commit string `json:"commit"` // full commit object ID, never a moving ref
}

// callNodeGit is the external Git boundary, replaced by a fail-loud fake during tests.
var callNodeGit = runNodeGit

// runNodeGit bounds Git execution and prevents an unattended credential prompt.
func runNodeGit(root string, args ...string) ([]byte, *ProcessError) {
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "git", args...) // #nosec G204 -- fixed executable; arguments are validated by nodeCheckout
	cmd.Dir = root
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0")
	cmd.WaitDelay = time.Second
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, processErr("git", err, "git in %q failed: %s", root, strings.TrimSpace(string(out)))
	}
	return out, nil
}

// nodeCheckout resolves a job base without creating a worktree, task metadata or runner.
// A private fetch ref isolates resolution from unrelated fetches and concurrent preparations.
func nodeCheckout(repos map[string]string, req NodeCheckoutRequest) (NodeCheckoutResult, *NodeCheckoutError) {
	if req.Repository == "" || req.BaseRef == "" {
		return NodeCheckoutResult{}, opErr(NodeCheckoutMissingArgument, "repository and base ref are required")
	}
	if !validJobIdentifier(req.Repository) {
		return NodeCheckoutResult{}, opErr(NodeCheckoutInvalidRepository, "repository %q must be a logical name, not a path", req.Repository)
	}
	root := repos[req.Repository]
	if root == "" {
		return NodeCheckoutResult{}, opErr(NodeCheckoutRepositoryNotAllowed, "repository %q is not configured in NODE_REPOS", req.Repository)
	}
	if !validJobBase(req.BaseRef, "") {
		return NodeCheckoutResult{}, opErr(NodeCheckoutInvalidBaseRef, "base %q is not a ref name", req.BaseRef)
	}
	if req.BaseSHA != "" && !validJobSHA(req.BaseSHA) {
		return NodeCheckoutResult{}, opErr(NodeCheckoutInvalidCommit, "commit must contain exactly 40 or 64 hexadecimal digits")
	}
	target := req.BaseRef
	sha := strings.ToLower(req.BaseSHA)
	if sha != "" {
		target = sha
	}
	var nonce [16]byte // random ref suffix isolates preparations across processes
	if _, err := rand.Read(nonce[:]); err != nil {
		return NodeCheckoutResult{}, fromStorage(NodeCheckoutStorage, storageErr(err, "cannot allocate a private checkout ref"))
	}
	privateRef := fmt.Sprintf("refs/team-agents/node-checkout/%x", nonce)
	defer func() {
		// A leftover private ref does not invalidate the resolved commit; report cleanup failures.
		if _, err := callNodeGit(root, "update-ref", "-d", privateRef); err != nil {
			warnSideEffect("remove node checkout ref", err, "ref", privateRef)
		}
	}()
	if _, err := callNodeGit(root, "fetch", "--no-tags", "--no-recurse-submodules", "--", "origin", target+":"+privateRef); err != nil {
		return NodeCheckoutResult{}, nodeCheckoutGitFailure(NodeCheckoutFetchFailed, err)
	}
	out, err := callNodeGit(root, "rev-parse", "--verify", privateRef+"^{commit}")
	if err != nil {
		return NodeCheckoutResult{}, nodeCheckoutGitFailure(NodeCheckoutCheckoutFailed, err)
	}
	commit := strings.ToLower(strings.TrimSpace(string(out)))
	if !validJobSHA(commit) || sha != "" && sha != commit {
		return NodeCheckoutResult{}, opErr(NodeCheckoutCheckoutFailed, "fetch did not resolve the requested commit")
	}
	return NodeCheckoutResult{Root: root, Commit: commit}, nil
}

// nodeCheckoutGitFailure preserves the process error while assigning this operation's cause.
func nodeCheckoutGitFailure(cause NodeCheckoutCause, err *ProcessError) *NodeCheckoutError {
	return &NodeCheckoutError{Cause: cause, Message: err.Error(), Err: err}
}

// nodeRepoPaths collects repeated local --repo-path declarations before validation.
type nodeRepoPaths []string

// String renders local mappings for flag help.
func (p *nodeRepoPaths) String() string { return strings.Join(*p, ",") }

// Set implements flag.Value; parseNodeCheckout validates all collected mappings.
func (p *nodeRepoPaths) Set(value string) error {
	*p = append(*p, value)
	return nil
}

// parseNodeCheckout parses local CLI flags and the explicitly supplied repository allow-list.
func parseNodeCheckout(args []string) (map[string]string, NodeCheckoutRequest, *NodeCheckoutError) {
	fs := newFlagSet("node-checkout")
	repo := fs.String("repo", "", "logical repository name in NODE_REPOS")
	base := fs.String("base", "", "base ref name")
	sha := fs.String("sha", "", "optional exact commit object ID")
	var paths nodeRepoPaths // explicitly supplied local repository configuration
	fs.Var(&paths, "repo-path", "local name=absolute-path mapping (repeatable)")
	if err := fs.Parse(args); err != nil {
		return nil, NodeCheckoutRequest{}, opErr(NodeCheckoutBadFlags, "%v", err)
	}
	if fs.NArg() != 0 {
		return nil, NodeCheckoutRequest{}, opErr(NodeCheckoutBadFlags, "node-checkout takes only flags")
	}
	repos := make(map[string]string)
	for _, entry := range paths {
		name, root, ok := strings.Cut(entry, "=")
		if !ok || !validJobIdentifier(name) || !filepath.IsAbs(root) || repos[name] != "" {
			return nil, NodeCheckoutRequest{}, opErr(NodeCheckoutBadFlags, "--repo-path must be a unique logical name=absolute-path mapping")
		}
		repos[name] = root
	}
	return repos, NodeCheckoutRequest{Repository: *repo, BaseRef: *base, BaseSHA: *sha}, nil
}

// cmdNodeCheckout is the thin local adapter used by shared node task preparation.
func cmdNodeCheckout(args []string) {
	repos, req, err := parseNodeCheckout(args)
	if err != nil {
		// The node persists this diagnostic as a job result; retain the operation-specific cause.
		err.Message = err.CauseName() + ": " + err.Message
		exitOn(err.asError())
		return
	}
	result, err := nodeCheckout(repos, req)
	if err != nil {
		// The node persists this diagnostic as a job result; retain the operation-specific cause.
		err.Message = err.CauseName() + ": " + err.Message
		exitOn(err.asError())
		return
	}
	exitOn(fromStorage(NodeCheckoutStorage, printJSON(result)).asError())
}
