package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"sort"
	"strings"
)

const (
	listingNormal       string = "normal"       // listingNormal identifies a full-capability team agent.
	listingWorker       string = "worker"       // listingWorker identifies an inexpensive helper.
	listingInvestigator string = "investigator" // listingInvestigator identifies a reviewer.
	cliClaude           string = "claude"       // cliClaude selects the Anthropic CLI provider.
	cliCodex            string = "codex"        // cliCodex selects the OpenAI CLI provider.
	cliGemini           string = "gemini"       // cliGemini selects the Google CLI provider.
	cliOpenCode         string = "opencode"     // cliOpenCode derives its provider from the model prefix.
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
		var names []string // names removes Go's map iteration randomness.
		for name := range cfg.Associative[group] {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			record := agentListing{Name: name, Type: cfg.Associative[group][name], Model: cfg.Associative["AGENT_MODEL"][name], Tier: listingNormal} // record starts as a normal agent.
			if group == "INVESTIGATORS_TYPE" {
				record.Model = cfg.Associative["INVESTIGATORS_MODEL"][name]
				record.Tier = listingInvestigator
			} else {
				for _, worker := range cfg.Indexed["WORKER_AGENTS"] {
					if worker == name {
						record.Tier = listingWorker
						break
					}
				}
				if p, ok := agentProviders[record.Type]; ok {
					record.Provider = p
				} else if record.Type == cliOpenCode {
					record.Provider, _, _ = strings.Cut(record.Model, "/")
				} else {
					record.Provider = "other"
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
		status, _ := meta.Lookup("STATUS")
		run, _ := meta.Lookup("RUN")
		if status == taskRunning || status == taskStarting {
			result, eerr := a.execute(context.Background(), Command{Name: "systemctl", Args: []string{"--user", "is-active", "--quiet", "agent-" + id + "-r" + run + ".service"}})
			if eerr != nil {
				return "", fromProcess(AgentsProcess, processErr("systemctl", eerr, "inspect task %s", id))
			}
			if result.ExitCode == 0 {
				return id, nil
			}
			// systemctl uses nonzero exit codes for inactive or unknown units, both expected answers here.
		}
		job, _ := meta.Lookup("JOB_ID")
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
			if allocation = "free"; id != "" {
				allocation = "busy: " + id
			}
		}
		model := record.Model // model uses a text-only default, unlike the JSON representation.
		if model == "" {
			model = "default"
		}
		if _, err := fmt.Fprintf(out, "%-12s %-7s model=%-24s %s\n", record.Name, record.Type, model, allocation); err != nil {
			return fromStorage(AgentsStorage, storageErr(err, "write agent row"))
		}
	}
	busBin := cfg.Scalars["AGENTBUS_BIN"] // busBin comes from the already resolved configuration.
	if busBin == "" {
		busBin = "agentbus"
	}
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

// cmdAgentsJSON emits one configuration record per agent and investigator.
func (a *App) cmdAgentsJSON(out io.Writer, cfg *Config) *AgentsJSONError {
	enc := json.NewEncoder(out) // enc emits one independently parseable object per line.
	enc.SetEscapeHTML(false)
	for _, record := range configuredListings(cfg) {
		if err := enc.Encode(record); err != nil {
			return fromStorage(AgentsJSONStorage, storageErr(err, "write agent JSON"))
		}
	}
	return nil
}
