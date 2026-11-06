package main

import (
	"context"
	"fmt"
	"io"
	"sort"
	"strings"
)

// CLI provider and listing tier constants classify agents across web and CLI displays.
const (
	listingNormal, listingWorker, listingInvestigator string = "normal", "worker", "investigator"      // agent listing tiers
	cliClaude, cliCodex, cliGemini, cliOpenCode       string = "claude", "codex", "gemini", "opencode" // supported agent CLI types
)

// agentListing is one line of the web panel's existing JSON interface.
type agentListing struct {
	Name     string `json:"name"`     // Name identifies the configured agent or investigator.
	Type     string `json:"type"`     // Type is the CLI used to start it.
	Model    string `json:"model"`    // Model stays empty when unspecified in configuration.
	Tier     string `json:"tier"`     // Tier distinguishes normal agents, workers and investigators.
	Provider string `json:"provider"` // Provider is empty for investigators, as in Bash.
}

// agentProviders maps known CLI names to their provider labels.
var agentProviders = map[string]string{cliClaude: "anthropic", cliCodex: "openai", cliGemini: "google"}

// configuredListings makes output deterministic while preserving the Bash field values.
func configuredListings(cfg *Config) []agentListing {
	var records []agentListing // records puts sorted investigators after the configured agents.
	for _, group := range []string{"AGENT_TYPE", "INVESTIGATORS_TYPE"} {
		var names []string
		for name := range cfg.Associative[group] {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			record := agentListing{Name: name, Type: cfg.Associative[group][name], Model: cfg.Associative["AGENT_MODEL"][name], Tier: listingNormal} // record starts as a normal agent.
			if group == "INVESTIGATORS_TYPE" {
				record.Model, record.Tier = cfg.Associative["INVESTIGATORS_MODEL"][name], listingInvestigator
			} else {
				for _, worker := range cfg.Indexed["WORKER_AGENTS"] {
					if worker == name {
						record.Tier = listingWorker
						break
					}
				}
				record.Provider = "other"
				if p, ok := agentProviders[record.Type]; ok {
					record.Provider = p
				} else if record.Type == cliOpenCode {
					record.Provider, _, _ = strings.Cut(record.Model, "/")
				}
			}
			records = append(records, record)
		}
	}
	return records
}

// runningTask returns the first Bash-compatible reservation; no match is an empty result.
func (a *App) runningTask(name, root string) (string, *AgentsError) {
	ids, serr := taskDirectories(root)
	if serr != nil {
		return "", fromStorage(AgentsStorage, serr)
	}
	for _, id := range ids {
		meta, merr := readTaskMeta(root, id)
		if merr != nil {
			if merr.Cause == ReadTaskMetaNotFound {
				continue
			}
			return "", fromStorage(AgentsStorage, storageErr(merr, "read task %s", id))
		}
		if agent, _ := meta.Lookup("AGENT"); agent != name {
			continue
		}
		status, run, job := metaLookup(meta, "STATUS", ""), metaLookup(meta, "RUN", ""), metaLookup(meta, "JOB_ID", "")
		if status == taskRunning || status == taskStarting {
			if res, err := a.execute(context.Background(), Command{Name: "systemctl", Args: []string{"--user", "is-active", "--quiet", "agent-" + id + "-r" + run + ".service"}}); err == nil && res.ExitCode == 0 { // a failed launch is the oracle's "not active" answer (unit_active: 2>/dev/null).
				return id, nil
			}
		}
		if status == taskSleeping || (job != "" && (run == "0" || run == "") && (status == taskNew || status == taskStarting || status == taskRunning)) {
			return id, nil
		}
	}
	return "", nil
}

// cmdAgents prints configured agents and current allocation using explicit state paths.
func (a *App) cmdAgents(out io.Writer, cfg *Config, root, busDir string) *AgentsError {
	for _, record := range configuredListings(cfg) {
		allocation := "investigator" // allocation is overridden only for ordinary agents.
		if record.Tier != listingInvestigator {
			id, aerr := a.runningTask(record.Name, root)
			if aerr != nil {
				return aerr
			}
			allocation = "free"
			if id != "" {
				allocation = "busy: " + id
			}
		}
		if _, err := fmt.Fprintf(out, "%-12s %-7s model=%-24s %s\n", record.Name, record.Type, firstNonEmpty(record.Model, "default"), allocation); err != nil {
			return fromStorage(AgentsStorage, storageErr(err, "write agent row"))
		}
	}
	busBin := firstNonEmpty(cfg.Scalars["AGENTBUS_BIN"], "agentbus") // busBin comes from the already resolved configuration.
	summary, berr := busCmd(context.Background(), a, busBin, "inv-list", "--dir", busDir, "--queue-max", cfg.Scalars["INVESTIGATION_QUEUE_MAX"], "--inv-slots", cfg.Scalars["INV_SLOTS"])
	if berr != nil {
		return fromProcess(AgentsProcess, processErr(busBin, berr, "list investigations"))
	}
	// head -1 preserves whether the first line was terminated in the bus output.
	if summary != "" {
		first, _, terminated := strings.Cut(summary, "\n")
		if terminated {
			first += "\n"
		}
		if _, err := io.WriteString(out, first); err != nil {
			return fromStorage(AgentsStorage, storageErr(err, "write investigation summary"))
		}
	}
	return nil
}
