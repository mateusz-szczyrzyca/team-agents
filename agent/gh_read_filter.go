package main

import (
	"encoding/json"
	"fmt"
	"strings"
)

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
//
//	(single object, possibly with nested comments/reviews arrays).
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
	// Build a set of trusted logins for O(1) lookups
	trustedSet := make(map[string]bool, len(trusted))
	for _, login := range trusted {
		trustedSet[login] = true
	}

	var data interface{}
	if err := json.Unmarshal(raw, &data); err != nil {
		return nil, nil, valueError("invalid JSON in gh response", err)
	}

	// isList is unused: all JSON structures supported regardless of list/detail type
	_ = isList
	var ignored []ghIgnoredEntry
	filtered := filterEntries(data, trustedSet, &ignored, false)

	result, err := json.Marshal(filtered)
	if err != nil {
		return nil, nil, valueError("failed to marshal filtered JSON", err)
	}

	return result, ignored, nil
}

// getAuthor extracts the author login from a comment/review entry, preferring author.login over user.login.
func getAuthor(entry map[string]interface{}) string {
	if author, ok := entry["author"].(map[string]interface{}); ok {
		if login, ok := author["login"].(string); ok {
			return login
		}
	}
	if user, ok := entry["user"].(map[string]interface{}); ok {
		if login, ok := user["login"].(string); ok {
			return login
		}
	}
	return ""
}

// getID extracts the ID from an entry, preferring id over databaseId.
func getID(entry map[string]interface{}) string {
	if id, ok := entry["id"]; ok {
		return toStr(id)
	}
	if id, ok := entry["databaseId"]; ok {
		return toStr(id)
	}
	return ""
}

// toStr converts various types to string for ID representation.
func toStr(v interface{}) string {
	switch vv := v.(type) {
	case string:
		return vv
	case float64:
		// JSON numbers are parsed as float64; preserve as integer string if whole number
		if vv == float64(int64(vv)) {
			return fmt.Sprintf("%d", int64(vv))
		}
		return fmt.Sprintf("%v", vv)
	default:
		return ""
	}
}

// getBody extracts the body and returns first 100 chars for logging.
func getBody(entry map[string]interface{}) string {
	body, ok := entry["body"].(string)
	if !ok {
		return ""
	}
	if len(body) > 100 {
		return body[:100]
	}
	return body
}

// isMarkedIgnore checks if body begins with "ignore this mate" (case-insensitive, allowing leading whitespace).
func isMarkedIgnore(body string) bool {
	trimmed := strings.TrimSpace(body)
	return strings.HasPrefix(strings.ToLower(trimmed), "ignore this mate")
}

// filterEntries recursively filters entries from the data structure.
func filterEntries(data interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry, isList bool) interface{} {
	switch v := data.(type) {
	case []interface{}:
		return filterArray(v, trusted, ignored)

	case map[string]interface{}:
		// Check if this is a comment/review object (has author/user and body)
		if isCommentEntry(v) {
			return filterEntry(v, trusted, ignored)
		}

		// Otherwise, it's a container; filter nested arrays
		result := make(map[string]interface{})
		for key, val := range v {
			if key == "comments" || key == "reviews" || key == "latestReviews" {
				if arr, ok := val.([]interface{}); ok {
					result[key] = filterArray(arr, trusted, ignored)
				} else {
					result[key] = val
				}
			} else {
				result[key] = val
			}
		}
		return result

	default:
		return data
	}
}

// isCommentEntry checks if an object looks like a comment/review entry.
// Per oracle: entry must have body and (author|user), but NOT have comments field.
func isCommentEntry(entry map[string]interface{}) bool {
	// Exclude objects that are containers (have comments field)
	if _, hasComments := entry["comments"]; hasComments {
		return false
	}
	if _, hasReviews := entry["reviews"]; hasReviews {
		return false
	}
	if _, hasLatestReviews := entry["latestReviews"]; hasLatestReviews {
		return false
	}

	hasBody := false
	hasAuthor := false

	if _, ok := entry["body"].(string); ok {
		hasBody = true
	}

	if author, ok := entry["author"].(map[string]interface{}); ok {
		if _, ok := author["login"]; ok {
			hasAuthor = true
		}
	} else if user, ok := entry["user"].(map[string]interface{}); ok {
		if _, ok := user["login"]; ok {
			hasAuthor = true
		}
	}

	return hasBody && hasAuthor
}

// filterEntry processes a single comment/review entry and returns it or nil based on trust/marked status.
func filterEntry(entry map[string]interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) interface{} {
	author := getAuthor(entry)
	body := getBody(entry)
	id := getID(entry)

	// Check precedence: untrusted author first
	if !trusted[author] {
		*ignored = append(*ignored, ghIgnoredEntry{
			Kind:   ghIgnoreKindComment, // default; caller could refine this
			Author: author,
			ID:     id,
			Body:   body,
			Reason: ghIgnoreReasonUntrustedAuthor,
		})
		return nil
	}

	// Check if marked "ignore this mate"
	if isMarkedIgnore(body) {
		*ignored = append(*ignored, ghIgnoredEntry{
			Kind:   ghIgnoreKindComment,
			Author: author,
			ID:     id,
			Body:   body,
			Reason: ghIgnoreReasonMarkedIgnore,
		})
		return nil
	}

	// Keep the entry
	return entry
}

// filterArray filters an array of entries, removing nil entries and condensing.
func filterArray(arr []interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) []interface{} {
	result := make([]interface{}, 0, len(arr))
	for _, item := range arr {
		if filtered := filterEntries(item, trusted, ignored, false); filtered != nil {
			result = append(result, filtered)
		}
	}
	return result
}

// valueError returns a ValueError with the given message and wrapped error.
func valueError(msg string, err error) *ValueError {
	return &ValueError{
		Message: msg,
		Err:     err,
	}
}
