package main

import (
	"encoding/json"
	"fmt"
	"strings"
)

// ghIgnoreKind types of entries filtered from GitHub responses.
type ghIgnoreKind string

const (
	// ghIgnoreKindComment represents a filtered comment.
	ghIgnoreKindComment = ghIgnoreKind("comment")
	// ghIgnoreKindReview represents a filtered review.
	ghIgnoreKindReview = ghIgnoreKind("review")
)

func (k ghIgnoreKind) Valid() bool {
	return k == ghIgnoreKindComment || k == ghIgnoreKindReview
}

// ghIgnoreReason why entries were filtered.
type ghIgnoreReason string

const (
	// ghIgnoreReasonUntrustedAuthor: entry author not in trusted list.
	ghIgnoreReasonUntrustedAuthor = ghIgnoreReason("untrusted_author")
	// ghIgnoreReasonMarkedIgnore: entry body starts with "ignore this mate".
	ghIgnoreReasonMarkedIgnore = ghIgnoreReason("marked_ignore")
)

func (r ghIgnoreReason) Valid() bool {
	return r == ghIgnoreReasonUntrustedAuthor || r == ghIgnoreReasonMarkedIgnore
}

// ghIgnoredEntry describes a filtered GitHub entry.
type ghIgnoredEntry struct {
	// Kind: comment or review.
	Kind ghIgnoreKind
	// Author: GitHub login.
	Author string
	// ID: entry ID.
	ID string
	// Body: first 100 chars for logging.
	Body string
	// Reason: why filtered.
	Reason ghIgnoreReason
}

// ghFilterJSON filters GitHub JSON keeping only trusted authors not marked "ignore this mate".
func ghFilterJSON(raw []byte, trusted []string, _ bool) ([]byte, []ghIgnoredEntry, *ValueError) {
	trustedSet := make(map[string]bool, len(trusted))
	for _, login := range trusted {
		trustedSet[login] = true
	}

	var data interface{}
	if err := json.Unmarshal(raw, &data); err != nil {
		return nil, nil, valueWrap(err, "invalid JSON")
	}

	var ignored []ghIgnoredEntry
	filtered := filter(data, trustedSet, &ignored)

	result, err := json.Marshal(filtered)
	if err != nil {
		return nil, nil, valueWrap(err, "marshal failed")
	}
	return result, ignored, nil
}

// filter recursively filters JSON structure, removing untrusted/marked entries.
func filter(data interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) interface{} {
	switch v := data.(type) {
	case []interface{}:
		var result []interface{}
		for _, item := range v {
			if filtered := filterItem(item, trusted, ignored); filtered != nil {
				result = append(result, filtered)
			}
		}
		return result

	case map[string]interface{}:
		// Check if this is a comment/review (has author/user, has body, NOT has comments/reviews)
		if isEntry(v) {
			return filterItem(v, trusted, ignored)
		}
		// Container: filter nested arrays
		result := make(map[string]interface{})
		for k, val := range v {
			if k == "comments" || k == "reviews" || k == "latestReviews" {
				if arr, ok := val.([]interface{}); ok {
					var filtered []interface{}
					for _, item := range arr {
						if f := filterItem(item, trusted, ignored); f != nil {
							filtered = append(filtered, f)
						}
					}
					result[k] = filtered
				} else {
					result[k] = val
				}
			} else {
				result[k] = val
			}
		}
		return result

	default:
		return data
	}
}

// filterItem processes a single entry, returning nil if filtered out.
func filterItem(item interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) interface{} {
	if m, ok := item.(map[string]interface{}); ok && isEntry(m) {
		author := getLogin(m)
		body, _ := m["body"].(string)
		id := getID(m)

		if !trusted[author] {
			*ignored = append(*ignored, ghIgnoredEntry{
				Kind: ghIgnoreKindComment, Author: author, ID: id, Body: body,
				Reason: ghIgnoreReasonUntrustedAuthor,
			})
			return nil
		}

		if isMarkedIgnore(body) {
			*ignored = append(*ignored, ghIgnoredEntry{
				Kind: ghIgnoreKindComment, Author: author, ID: id, Body: body,
				Reason: ghIgnoreReasonMarkedIgnore,
			})
			return nil
		}
	}
	return item
}

// isEntry checks if object is a comment/review (has author, NOT a container).
func isEntry(m map[string]interface{}) bool {
	// Exclude containers
	if _, ok := m["comments"]; ok {
		return false
	}
	if _, ok := m["reviews"]; ok {
		return false
	}
	if _, ok := m["latestReviews"]; ok {
		return false
	}
	// Must have author or user field
	hasAuthor := false
	if a, ok := m["author"].(map[string]interface{}); ok {
		if _, ok := a["login"]; ok {
			hasAuthor = true
		}
	}
	if u, ok := m["user"].(map[string]interface{}); ok {
		if _, ok := u["login"]; ok {
			hasAuthor = true
		}
	}
	return hasAuthor
}

// getLogin extracts author login (prefers author.login over user.login).
func getLogin(m map[string]interface{}) string {
	if a, ok := m["author"].(map[string]interface{}); ok {
		if l, ok := a["login"].(string); ok {
			return l
		}
	}
	if u, ok := m["user"].(map[string]interface{}); ok {
		if l, ok := u["login"].(string); ok {
			return l
		}
	}
	return ""
}

// getID extracts id or databaseId as string.
func getID(m map[string]interface{}) string {
	if id, ok := m["id"]; ok {
		return idString(id)
	}
	if id, ok := m["databaseId"]; ok {
		return idString(id)
	}
	return ""
}

// idString converts ID values to string.
func idString(v interface{}) string {
	switch x := v.(type) {
	case string:
		return x
	case float64:
		// Preserve numeric IDs as strings (JSON parses numbers as float64)
		if x == float64(int64(x)) {
			return fmt.Sprintf("%d", int64(x))
		}
		return fmt.Sprintf("%v", x)
	default:
		return ""
	}
}

// isMarkedIgnore checks for "ignore this mate" (case-insensitive, leading whitespace).
func isMarkedIgnore(body string) bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimSpace(body)), "ignore this mate")
}
