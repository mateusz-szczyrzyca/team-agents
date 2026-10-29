package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"io"
	"os"
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

// read emits the new complete events; without overlap it emits the whole available tail.
func (c *mirrorActivityCursor) read(path string) []Activity {
	lines, ok := mirrorActivityLines(path)
	if !ok {
		return nil
	}
	overlap := mirrorActivityOverlap(c.previous, lines)
	var activities []Activity
	for _, line := range lines[overlap:] {
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
