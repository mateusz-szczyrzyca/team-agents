package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

// ghIgnoreKind types of entries filtered from GitHub responses.
type ghIgnoreKind string

const (
	ghIgnoreKindComment = ghIgnoreKind("comment") // represents a filtered comment
	ghIgnoreKindReview  = ghIgnoreKind("review")  // represents a filtered review
)

func (k ghIgnoreKind) Valid() bool { return k == ghIgnoreKindComment || k == ghIgnoreKindReview }

// ghIgnoreReason why entries were filtered.
type ghIgnoreReason string

const (
	ghIgnoreReasonUntrustedAuthor = ghIgnoreReason("untrusted_author") // author not in trusted list
	ghIgnoreReasonMarkedIgnore    = ghIgnoreReason("marked_ignore")    // body starts with "ignore this mate"
)

func (r ghIgnoreReason) Valid() bool {
	return r == ghIgnoreReasonUntrustedAuthor || r == ghIgnoreReasonMarkedIgnore
}

// ghIgnoredEntry describes a filtered GitHub entry.
type ghIgnoredEntry struct {
	Kind   ghIgnoreKind   // comment or review
	Author string         // GitHub login
	ID     string         // entry ID
	Body   string         // first 100 chars for logging
	Reason ghIgnoreReason // why filtered
}

// ghFilterJSON filters GitHub JSON keeping only trusted authors not marked "ignore this mate".
func ghFilterJSON(raw []byte, trusted []string, _ bool) ([]byte, []ghIgnoredEntry, *ValueError) {
	trustedSet := make(map[string]bool, len(trusted))
	for _, login := range trusted {
		trustedSet[login] = true
	}
	var data interface{}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber() // keeps large numeric IDs exact as json.Number instead of float64
	if err := dec.Decode(&data); err != nil {
		return nil, nil, valueWrap(err, "invalid JSON")
	}
	var ignored []ghIgnoredEntry
	filtered := filter(data, trustedSet, &ignored)
	result, err := json.Marshal(filtered)
	if err != nil {
		return nil, nil, valueWrap(err, "marshal failed")
	}
	// A fully ignored standalone object filters to nil; emit empty bytes, not "null".
	if filtered == nil {
		return []byte{}, ignored, nil
	}
	return result, ignored, nil
}

// filter recursively filters JSON structure, removing untrusted/marked entries.
func filter(data interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) interface{} {
	switch v := data.(type) {
	case []interface{}:
		// Start non-nil so an all-filtered array marshals as [] not null.
		result := []interface{}{}
		for _, item := range v {
			if arr, ok := item.([]interface{}); ok {
				result = append(result, filter(arr, trusted, ignored))
				continue
			}
			if filtered := filterItem(item, trusted, ignored); filtered != nil {
				result = append(result, filtered)
			}
		}
		return result
	case map[string]interface{}:
		if isEntry(v) {
			return filterItem(v, trusted, ignored)
		}
		// Container: reuse filter on nested comment arrays so an all-rejected one stays [].
		result := make(map[string]interface{})
		for k, val := range v {
			if arr, ok := val.([]interface{}); ok && (k == "comments" || k == "reviews" || k == "latestReviews") {
				val = filter(arr, trusted, ignored)
			}
			result[k] = val
		}
		return result
	}
	return data
}

// filterItem returns nil for a rejected entry. Every map from an array is
// trust-checked: a missing or null author yields "" and fails the trusted lookup.
func filterItem(item interface{}, trusted map[string]bool, ignored *[]ghIgnoredEntry) interface{} {
	m, ok := item.(map[string]interface{})
	if !ok {
		return item
	}
	author := getLogin(m)
	body, _ := m["body"].(string)
	id := getID(m)
	reject := func(reason ghIgnoreReason) interface{} {
		*ignored = append(*ignored, ghIgnoredEntry{Kind: ghIgnoreKindComment, Author: author, ID: id, Body: body, Reason: reason})
		return nil
	}
	if !trusted[author] {
		return reject(ghIgnoreReasonUntrustedAuthor)
	}
	if isMarkedIgnore(body) {
		return reject(ghIgnoreReasonMarkedIgnore)
	}
	return m
}

// isEntry checks if object is a comment/review (has body, NOT a container).
// Recognizes comments independently of login validity so null/missing authors are still filtered.
func isEntry(m map[string]interface{}) bool {
	// Not a comment if it's a container (has comments, reviews, or latestReviews array)
	for _, key := range []string{"comments", "reviews", "latestReviews"} {
		if _, ok := m[key]; ok {
			return false
		}
	}
	// Is a comment if it has a body field (even without author or with null author)
	_, hasBody := m["body"].(string)
	return hasBody
}

// getLogin extracts author login (prefers author.login over user.login).
func getLogin(m map[string]interface{}) string {
	for _, field := range []string{"author", "user"} {
		if f, ok := m[field].(map[string]interface{}); ok {
			if l, ok := f["login"].(string); ok {
				return l
			}
		}
	}
	return ""
}

// getID extracts id or databaseId as string; a null or empty id falls back to databaseId.
func getID(m map[string]interface{}) string {
	if s := idString(m["id"]); s != "" {
		return s
	}
	return idString(m["databaseId"])
}

// idString converts ID values to string.
func idString(v interface{}) string {
	switch x := v.(type) {
	case string:
		return x
	case json.Number:
		return x.String() // exact text as it appeared in the JSON
	case float64: // JSON decodes numbers without UseNumber as float64
		if x == float64(int64(x)) { // integral IDs print without exponent
			return fmt.Sprintf("%d", int64(x))
		}
		return fmt.Sprintf("%v", x)
	}
	return ""
}

// isMarkedIgnore checks for "ignore this mate" (case-insensitive, leading whitespace).
func isMarkedIgnore(body string) bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimSpace(body)), "ignore this mate")
}
