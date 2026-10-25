package main

import (
	"encoding/hex"
	"strings"
)

// validJobIdentifier accepts one bounded logical name, never a filesystem path.
func validJobIdentifier(s string) bool {
	if len(s) == 0 || len(s) > 128 || !jobAlphanumeric(s[0]) {
		return false
	}
	for i := 1; i < len(s); i++ {
		if !jobAlphanumeric(s[i]) && !strings.ContainsRune("._-", rune(s[i])) {
			return false
		}
	}
	return true
}

// jobAlphanumeric is deliberately ASCII: logical identifiers cross URL and config boundaries.
func jobAlphanumeric(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9'
}

// validJobBase checks a Git ref name and an optional exact object ID without running Git.
func validJobBase(ref, sha string) bool {
	if ref == "" || ref == "@" || strings.HasPrefix(ref, "-") || strings.HasSuffix(ref, ".") ||
		strings.Contains(ref, "..") || strings.Contains(ref, "@{") {
		return false
	}
	for _, c := range ref {
		if c <= ' ' || c == 127 || strings.ContainsRune("~^:?*[\\", c) {
			return false
		}
	}
	for _, component := range strings.Split(ref, "/") {
		if component == "" || strings.HasPrefix(component, ".") || strings.HasSuffix(component, ".lock") {
			return false
		}
	}
	return sha == "" || validJobSHA(sha)
}

// validJobSHA accepts a complete SHA-1 or SHA-256 object ID, without a ref name.
func validJobSHA(sha string) bool {
	if len(sha) != 40 && len(sha) != 64 {
		return false
	}
	_, err := hex.DecodeString(sha)
	return err == nil
}
