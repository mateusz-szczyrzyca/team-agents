package main

import "context"

// leadCall runs one request on the lead instance's own hub, in-process and without HTTP (design section 2:
// the lead's own agents call the hub directly). instance is the lead's INSTANCE_NAME; the hub checks that
// the caller claims it. Execute returns Pending when a poll interval passes, so leadCall repeats the same
// request, which reattaches to the running operation with the same request ID, until a final response
// arrives or ctx ends. In the second case the Pending response is returned and the caller decides what to do.
func leadCall(ctx context.Context, h *Hub, instance string, req HubRequest) HubResponse {
	for {
		// Execute's status code only mirrors the HTTP answer; the in-process caller reads the response body.
		response, _ := h.Execute(ctx, instance, req)
		if response.Pending == nil || ctx.Err() != nil {
			return response
		}
	}
}
