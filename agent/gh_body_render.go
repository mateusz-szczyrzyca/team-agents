package main

// GitHubColorConfig carries the runner's raw color environment without registry IO.
type GitHubColorConfig struct {
	// NameColors is AGENT_GITHUB_NAME_COLORS: whitespace-separated name=color pairs.
	NameColors string
	// Palette is AGENT_GITHUB_COLORS: ordered whitespace-separated fallback colors.
	Palette string
}

// RenderGitHubNames colors attribution and investigator names like gh_colour_names.
func RenderGitHubNames(_ string, _ GitHubColorConfig) string { return "" }

// RenderGitHubRecommendation colors the first eligible verdict in a review.
func RenderGitHubRecommendation(_ string) string { return "" }

// TransformPublishBody removes generated attribution before coloring agent names.
func TransformPublishBody(_ string, _ GitHubColorConfig) string { return "" }

// TransformReviewBody inserts the byline, colors the verdict, then publishes the body.
func TransformReviewBody(_, _ string, _ GitHubColorConfig) string { return "" }

// gitHubNameChecksum computes the POSIX cksum value used for palette fallback.
func gitHubNameChecksum(_ string) uint32 { return 0 }
