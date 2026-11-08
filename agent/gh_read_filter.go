package main

// ghIgnoreKind: the type of entry that was filtered from GitHub JSON responses.
type ghIgnoreKind string

const (
	// ghIgnoreKindComment: a GitHub comment was filtered.
	ghIgnoreKindComment = ghIgnoreKind("comment")
	// ghIgnoreKindReview: a GitHub review was filtered.
	ghIgnoreKindReview = ghIgnoreKind("review")
)

// Valid returns true if the kind is a recognized value.
func (k ghIgnoreKind) Valid() bool {
	return k == ghIgnoreKindComment || k == ghIgnoreKindReview
}

// ghIgnoreReason: why an entry was filtered from GitHub JSON responses.
type ghIgnoreReason string

const (
	// ghIgnoreReasonUntrustedAuthor: the entry author is not in the trusted list.
	ghIgnoreReasonUntrustedAuthor = ghIgnoreReason("untrusted_author")
	// ghIgnoreReasonMarkedIgnore: the entry body begins with "ignore this mate" (case-insensitive).
	ghIgnoreReasonMarkedIgnore = ghIgnoreReason("marked_ignore")
)

// Valid returns true if the reason is a recognized value.
func (r ghIgnoreReason) Valid() bool {
	return r == ghIgnoreReasonUntrustedAuthor || r == ghIgnoreReasonMarkedIgnore
}

// ghIgnoredEntry describes a GitHub entry (comment or review) that was filtered from a response.
type ghIgnoredEntry struct {
	// Kind: what type of entry was filtered (comment or review).
	Kind ghIgnoreKind
	// Author: the author's GitHub login (author.login or user.login from the JSON).
	Author string
	// ID: the entry's unique identifier (id or databaseId from the JSON).
	ID string
	// Body: excerpt of the entry body, for logging purposes (first 100 characters).
	Body string
	// Reason: why the entry was filtered (untrusted author or marked "ignore this mate").
	Reason ghIgnoreReason
}

// ghFilterJSON filters GitHub API JSON responses, keeping only entries from trusted authors
// that are not marked "ignore this mate". It returns the filtered JSON, a list of filtered
// entries with details for logging, and any parsing error.
//
// trusted: list of GitHub account logins (author.login or user.login) that are trusted.
// isList: true for list-type responses (array of top-level items), false for detail responses
//         (single object, possibly with nested comments/reviews arrays).
//
// Filtering rules:
// - Keep an entry if: author is in trusted AND body does not begin with "ignore this mate" (case-insensitive).
// - Remove if author is not trusted (reason: untrusted_author).
// - Remove if body begins with "ignore this mate" (reason: marked_ignore), even if trusted.
// - Precedence: untrusted author is checked before marked ignore (untrusted entries logged first).
//
// Supported JSON structures:
// - Array of comments/reviews at top level (list responses).
// - Single comment/review object at top level (detail responses).
// - Object with nested .comments, .reviews, .latestReviews arrays (PR/issue detail).
// - Pagination: each page's array is filtered independently.
func ghFilterJSON(raw []byte, trusted []string, isList bool) ([]byte, []ghIgnoredEntry, *ValueError) {
	// Placeholder: implementation in next commit
	return nil, nil, nil
}
