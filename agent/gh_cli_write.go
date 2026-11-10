package main

import (
	"context"
)

// handleCLIWrite handles CLI write forwarding with body transformation for pr/issue create/edit/comment.
func (p GHDispatchParams) handleCLIWrite(_ context.Context, _ string) (int, *GHReadDispatchError) {
	return 0, nil
}
