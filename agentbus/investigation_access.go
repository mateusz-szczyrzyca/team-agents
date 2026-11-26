// Investigation tool access for one MCP server run.
package main

// investigationTools controls whether a run may use investigation tools; zero preserves legacy access.
type investigationTools uint8

const (
	investigationsLegacy investigationTools = iota // an old daemon retains the legacy role checks
	investigationsOn                               // a team task may use investigation tools regardless of its agent tier
	investigationsOff                              // the daemon excluded this run from investigation tools
)

// String renders the flag's default for help.
func (v *investigationTools) String() string {
	if *v == investigationsOff {
		return "off"
	}
	return "on"
}

// Set implements flag.Value; failures contain a *ValueError describing invalid access.
func (v *investigationTools) Set(raw string) error {
	switch raw {
	case "on":
		*v = investigationsOn
	case "off":
		*v = investigationsOff
	default:
		return valueErr("investigations must be off or on, got %q", raw)
	}
	return nil
}

// investigationNotAllowedMessage explains which task roles the daemon permits to investigate.
const investigationNotAllowedMessage = "only the coordinator and the members of a team may request an investigation"
