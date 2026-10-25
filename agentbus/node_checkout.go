package main

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
var callNodeGit = func(_ string, _ ...string) ([]byte, *ProcessError) { return nil, nil }

// nodeCheckout resolves a job base without creating a worktree, task metadata or runner.
func nodeCheckout(_ map[string]string, _ NodeCheckoutRequest) (NodeCheckoutResult, *NodeCheckoutError) {
	return NodeCheckoutResult{}, nil
}

// parseNodeCheckout parses local CLI flags and the explicitly supplied repository allow-list.
func parseNodeCheckout(_ []string) (map[string]string, NodeCheckoutRequest, *NodeCheckoutError) {
	return nil, NodeCheckoutRequest{}, nil
}
