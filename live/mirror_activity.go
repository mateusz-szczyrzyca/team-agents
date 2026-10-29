package main

import (
	"bufio"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
)

// mirrorActivityCursor retains the last complete bounded tail, since sync replaces rather than appends it.
type mirrorActivityCursor struct {
	previous []string // complete raw JSON lines already consumed, including unknown event fields
}

// mirrorActivityLines reads one opened generation; incomplete trailing lines remain pending until the next sync.
func mirrorActivityLines(path string) ([]string, bool) {
	f, err := os.Open(path) // #nosec G304 -- callers construct the mirror path from locally enumerated task directories
	if errors.Is(err, os.ErrNotExist) {
		return nil, false
	}
	if err != nil {
		warnOnce("mirror-activity:"+path, LogFileReadFailed, storageErr(err, "cannot open %s", path), "file", path)
		return nil, false
	}
	defer closeQuietly(f, path)
	var lines []string
	reader := bufio.NewReader(f)
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			if !errors.Is(err, io.EOF) {
				warnOnce("mirror-activity-read:"+path, LogFileReadFailed, storageErr(err, "cannot read %s", path), "file", path)
				return nil, false
			}
			return lines, true
		}
		lines = append(lines, line)
	}
}

// baseline records only complete lines through the page's byte offset; negative offsets consume the whole tail.
func (c *mirrorActivityCursor) baseline(path string, off int64) {
	lines, ok := mirrorActivityLines(path)
	if !ok {
		return
	}
	var consumed int64
	for _, line := range lines {
		consumed += int64(len(line))
		if off >= 0 && consumed > off {
			break
		}
		c.previous = append(c.previous, line)
	}
}

// mirrorActivityPrefixDigest hashes the sequence of lines preceding the boundary.
func mirrorActivityPrefixDigest(lines []string) string {
	sum := sha256.Sum256([]byte(strings.Join(lines, "")))
	return fmt.Sprintf("%x", sum)
}

// token identifies the last complete raw line consumed from the generation rendered on the page,
// paired with a digest of preceding rendered lines to validate continuity across replacements.
// "start" represents an empty or not-yet-created log, so its first batch is never skipped.
func (c *mirrorActivityCursor) token() string {
	if len(c.previous) == 0 {
		return "start"
	}
	boundary := mirrorActivityToken(c.previous[len(c.previous)-1])
	prefixDigest := mirrorActivityPrefixDigest(c.previous[:len(c.previous)-1])
	return boundary + ":" + prefixDigest
}

// mirrorActivityToken hashes all event fields, preserving timestamps and unknown fields in the handoff.
func mirrorActivityToken(line string) string {
	return fmt.Sprintf("%x", sha256.Sum256([]byte(line)))
}

// resume binds a page's content cursor to the current tail. If its boundary has rolled out,
// is ambiguous, or context continuity cannot be validated, replay the available tail
// rather than consuming unseen records using an obsolete byte offset.
func (c *mirrorActivityCursor) resume(path, token string) {
	c.previous = nil
	if token == "" || token == "start" {
		return
	}
	lines, ok := mirrorActivityLines(path)
	if !ok {
		return
	}
	hash := token
	prefixDigest := ""
	if idx := strings.IndexByte(token, ':'); idx >= 0 {
		hash = token[:idx]
		prefixDigest = token[idx+1:]
	}

	matchCount := 0
	matchIdx := -1
	for i, line := range lines {
		if mirrorActivityToken(line) == hash {
			matchCount++
			matchIdx = i
		}
	}
	// Unique match: unambiguous boundary candidate.
	// Multiple matches: ambiguous boundary in truncated/duplicate tail; replay available tail.
	// Zero matches: rolled out boundary; replay available tail.
	if matchCount == 1 {
		if matchIdx > 0 {
			if prefixDigest == "" || mirrorActivityPrefixDigest(lines[:matchIdx]) != prefixDigest {
				return
			}
		}
		c.previous = lines[:matchIdx+1]
	}
}

// mirrorActivityOverlap finds the longest old suffix matching the new prefix in linear time.
// Comparing raw lines preserves repeated text with distinct timestamps and unknown fields.
func mirrorActivityOverlap(previous, next []string) int {
	if len(next) == 0 {
		return 0
	}
	prefix := make([]int, len(next))
	for i, j := 1, 0; i < len(next); i++ {
		for j > 0 && next[i] != next[j] {
			j = prefix[j-1]
		}
		if next[i] == next[j] {
			j++
		}
		prefix[i] = j
	}
	matched := 0
	for _, line := range previous {
		if matched == len(next) {
			matched = prefix[matched-1]
		}
		for matched > 0 && line != next[matched] {
			matched = prefix[matched-1]
		}
		if line == next[matched] {
			matched++
		}
	}
	return matched
}

// isCompleteActivity verifies whether a line is a valid JSON activity entry.
func isCompleteActivity(line string) bool {
	var a Activity
	return json.Unmarshal([]byte(line), &a) == nil
}

// read emits the new complete events; without overlap it emits the whole available tail.
func (c *mirrorActivityCursor) read(path string) []Activity {
	lines, ok := mirrorActivityLines(path)
	if !ok {
		return nil
	}
	leadingFragment := false
	if len(lines) > 0 && !isCompleteActivity(lines[0]) {
		leadingFragment = true
		var a Activity
		err := json.Unmarshal([]byte(lines[0]), &a)
		warnOnce("activity:"+path+":"+lines[0], LogCorruptActivityLine, valueWrap(err, "corrupt line in %s", path), "file", path)
	}

	prevValid := c.previous
	if len(prevValid) > 0 && !isCompleteActivity(prevValid[0]) {
		prevValid = prevValid[1:]
	}
	nextValid := lines
	if leadingFragment {
		nextValid = nextValid[1:]
	}

	overlap := mirrorActivityOverlap(prevValid, nextValid)
	emitStart := overlap
	if leadingFragment {
		emitStart = 1 + overlap
	}

	var activities []Activity
	for _, line := range lines[emitStart:] {
		var activity Activity
		if err := json.Unmarshal([]byte(line), &activity); err != nil {
			warnOnce("activity:"+path+":"+line, LogCorruptActivityLine, valueWrap(err, "corrupt line in %s", path), "file", path)
			continue
		}
		activities = append(activities, activity)
	}
	c.previous = lines
	return activities
}
