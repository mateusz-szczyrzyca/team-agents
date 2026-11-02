// help.go provides help and usage text generation.
package main

// HelpProvider generates help text for the agent command.
type HelpProvider struct{}

// NewHelpProvider creates a new help provider.
func NewHelpProvider() *HelpProvider {
	return &HelpProvider{}
}

// HelpAliases returns the recognized help command aliases.
func (h *HelpProvider) HelpAliases() []string {
	return []string{"-h", "--help", "help"}
}

// IsHelpAlias returns true if the given string is a help alias.
func (h *HelpProvider) IsHelpAlias(s string) bool {
	for _, alias := range h.HelpAliases() {
		if s == alias {
			return true
		}
	}
	return false
}
