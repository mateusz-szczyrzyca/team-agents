package main

import (
	"context"
	"io"
	"os"
	"slices"
	"strings"
)

// handleCLIWrite handles CLI write forwarding with body transformation for pr/issue create/edit/comment.
// nolint:unparam // realGH retained for interface consistency with potential varied executables
func (p GHDispatchParams) handleCLIWrite(ctx context.Context, realGH string) (int, *GHReadDispatchError) {
	// Guard: only transform body args for pr/issue create/edit/comment
	if len(p.Args) < 2 ||
		(p.Args[0] != "pr" && p.Args[0] != "issue") ||
		(p.Args[1] != "create" && p.Args[1] != "edit" && p.Args[1] != "comment") {
		return p.run(ctx, realGH, p.Args, p.Stdin, p.Stdout, p.Stderr)
	}

	args := slices.Clone(p.Args)
	var tmpFile string
	stdinReader := p.Stdin

	// Prepare color config from environment
	cfg := GitHubColorConfig{
		NameColors: ghEnvValue(p.Env, "AGENT_GITHUB_NAME_COLORS"),
		Palette:    ghEnvValue(p.Env, "AGENT_GITHUB_COLORS"),
	}

	// Cleanup ensures temp file is removed after execution
	defer func() {
		if tmpFile != "" {
			if removeErr := os.Remove(tmpFile); removeErr != nil && !os.IsNotExist(removeErr) {
				p.log(LogCommandFailed, "temp file cleanup failed", map[string]string{"reason": removeErr.Error()})
			}
		}
	}()

	// Process arguments for body transformations
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--body" || arg == "-b":
			// Transform next argument if it exists, strip trailing LF
			if i+1 < len(args) {
				transformed := TransformPublishBody(args[i+1], cfg)
				args[i+1] = strings.TrimRight(transformed, "\n")
			}

		case len(arg) > 7 && arg[:7] == "--body=":
			// Transform value part of --body=value, strip trailing LF
			value := arg[7:]
			transformed := TransformPublishBody(value, cfg)
			args[i] = "--body=" + strings.TrimRight(transformed, "\n")

		case arg == "--body-file" || arg == "-F":
			// Only process first body-file
			if tmpFile != "" {
				continue
			}

			// Get the file path
			if i+1 >= len(args) {
				continue
			}
			filePath := args[i+1]

			// Create temp file
			tmp, tmpfileErr := os.CreateTemp("", "gh_body_*.txt")
			if tmpfileErr != nil {
				// Log temp creation failure and break loop, forward current args
				p.log(EventGitHubNamesFailed, "temporary file creation failed", map[string]string{"reason": tmpfileErr.Error()})
				break
			}
			tmpFileName := tmp.Name()
			if closeErr := tmp.Close(); closeErr != nil {
				p.log(LogCommandFailed, "temp file close failed", map[string]string{"reason": closeErr.Error()})
			}

			// Read from stdin or file
			var bodyText string
			if filePath == "-" {
				// Read from stdin
				data, readErr := io.ReadAll(stdinReader)
				if readErr != nil {
					// Cleanup and retain original path in args
					if removeErr := os.Remove(tmpFileName); removeErr != nil {
						p.log(LogCommandFailed, "temp file cleanup failed", map[string]string{"reason": removeErr.Error()})
					}
					continue
				}
				bodyText = string(data)
				// stdin is now consumed, don't pass it to executor
				stdinReader = strings.NewReader("")
			} else {
				// Read from file: user-provided path via --body-file
				data, readErr := os.ReadFile(filePath) // #nosec G304 -- path provided via --body-file forwarded to gh
				if readErr != nil {
					// Cleanup and retain original path in args - let real gh fail with proper error
					if removeErr := os.Remove(tmpFileName); removeErr != nil {
						p.log(LogCommandFailed, "temp file cleanup failed", map[string]string{"reason": removeErr.Error()})
					}
					continue
				}
				bodyText = string(data)
			}

			// Transform the body text, preserve LF for file bodies
			transformed := TransformPublishBody(bodyText, cfg)

			// Write to temp file
			if writeErr := os.WriteFile(tmpFileName, []byte(transformed), 0600); writeErr != nil { // #nosec G703 -- tmpFileName from os.CreateTemp
				// Cleanup and continue unchanged
				if removeErr := os.Remove(tmpFileName); removeErr != nil {
					p.log(LogCommandFailed, "temp file cleanup failed", map[string]string{"reason": removeErr.Error()})
				}
				continue
			}

			// Use temp file path
			args[i+1] = tmpFileName
			tmpFile = tmpFileName
		}
	}

	// Execute the real gh command with transformed args
	return p.run(ctx, realGH, args, stdinReader, p.Stdout, p.Stderr)
}
