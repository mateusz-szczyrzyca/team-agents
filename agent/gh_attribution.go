package main

import "strings"

// FormatAttributionBlock renders the Bash attribution footer with a final newline.
// taskMetas contains parsed on-disk records in task-path order, including duplicates.
// It remains independent of GitHub writes and performs no filesystem operations.
func FormatAttributionBlock(indent, agent, team string, taskMetas []MetaFile, config Config) string {
	lines := []string{indent + "Opened by team-agents:"}
	descriptor := FormatAgentDescriptor(agent, &config)
	if team == "" {
		return strings.Join(append(lines, indent+"- agent: "+descriptor), "\n") + "\n"
	}
	lines = append(lines, indent+"- team: `"+team+"`", indent+"- coordinator: "+descriptor)
	for _, meta := range taskMetas {
		member := attributionMember(meta, team)
		if member != "" && member != agent {
			lines = append(lines, indent+"- member: "+FormatAgentDescriptor(member, &config))
		}
	}
	return strings.Join(lines, "\n") + "\n"
}

// attributionMember follows grep/sed on physical source lines rather than decoded
// metadata values: duplicate assignments join with newlines, as in Bash substitution.
func attributionMember(meta MetaFile, team string) string {
	var source strings.Builder
	for _, line := range meta.Lines {
		source.WriteString(line.Raw)
	}
	matched := false
	var names []string
	for _, line := range strings.Split(source.String(), "\n") {
		if line == "TEAM="+team {
			matched = true
		}
		if strings.HasPrefix(line, "AGENT=") {
			names = append(names, strings.TrimPrefix(line, "AGENT="))
		}
	}
	if !matched {
		return ""
	}
	return strings.TrimRight(strings.Join(names, "\n"), "\n")
}
