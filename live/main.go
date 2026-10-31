// agentlive — live view of the agents in ~/agents (started by `agent live`).
//
//	agentlive --home ~/agents --agent ~/.local/bin/agent [--remind 1m] [--queue-max 3] [--feed]
//	agentlive web [--listen 127.0.0.1:8787] [--users login,…] …
//
// Default mode: full-screen panel (Bubble Tea). --feed: event stream on stdout. web: the web interface (web.go).
package main

import (
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// config holds the command-line settings of agentlive (passed by `agent live`).
type config struct {
	home     string        // AGENTS_HOME (tasks/, bus/)
	busDir   string        // agentbus state directory (default <home>/bus)
	agentCmd string        // path to the `agent` script, used for actions (answer, stop, say, ack)
	remind   time.Duration // interval of "still working / waiting for you" reminders (0 = off)
	queueMax int           // investigation queue limit, for the queue summary line

	remoteControls bool // lead-side answer and kick commands are configured by the launcher
}

// main parses flags and starts the panel, the feed or (agentlive web …) the web interface.
func main() {
	if len(os.Args) > 1 && os.Args[1] == "web" {
		runWebCmd(os.Args[2:])
		return
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fmt.Fprintln(os.Stderr, "agentlive: no home directory:", err)
		os.Exit(2)
	}
	cfg := config{}
	flag.StringVar(&cfg.home, "home", filepath.Join(home, "agents"), "AGENTS_HOME directory")
	flag.StringVar(&cfg.busDir, "bus", "", "bus directory (default <home>/bus)")
	flag.StringVar(&cfg.agentCmd, "agent", filepath.Join(home, ".local", "bin", "agent"), "path to the agent script")
	remindMin := flag.Float64("remind", 1, "remind every N minutes who is still working (0 = off)")
	flag.IntVar(&cfg.queueMax, "queue-max", 3, "investigation queue limit")
	flag.IntVar(&InvSlots, "inv-slots", 1, "number of investigators")
	feed := flag.Bool("feed", false, "event stream mode (no panel)")
	light := flag.Bool("light", false, "colours for a light terminal background")
	flag.Parse()
	// Explicit background: otherwise Lip Gloss queries the terminal (OSC 11) and swallows keys until it answers.
	lipgloss.SetHasDarkBackground(!*light)
	if cfg.busDir == "" {
		cfg.busDir = filepath.Join(cfg.home, "bus")
	}
	cfg.remind = time.Duration(*remindMin * float64(time.Minute))
	role := "panel"
	if *feed {
		role = "feed"
	}
	setupLogging(cfg.home, role)

	if *feed {
		runFeed(cfg)
		return
	}
	p := tea.NewProgram(newModel(cfg), tea.WithAltScreen())
	if _, err := p.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "agentlive:", err)
		os.Exit(1)
	}
}

// runFeed prints events to stdout until interrupted (agent live --feed).
func runFeed(cfg config) {
	w := NewWatcher(cfg.home, cfg.busDir, cfg.remind)
	snap, _ := w.Poll() // the first poll only records the baseline: it has no events
	printSummary(cfg, snap)
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	tk := time.NewTicker(time.Second)
	defer tk.Stop()
	for {
		select {
		case <-sig:
			return
		case <-tk.C:
			snap, ev := w.Poll()
			for _, e := range ev {
				fmt.Println(fmtEvent(e))
				if e.Kind == "remind" {
					fmt.Println(stDim.Render("         " + snap.QueueLine(cfg.queueMax)))
				}
			}
		}
	}
}

// printSummary prints the initial task list in feed mode.
func printSummary(cfg config, s *Snapshot) {
	fmt.Println(stHeader.Render("agent live --feed") + "  " + stDim.Render(s.At.Format("2006-01-02 15:04:05")) + "  │ " + s.QueueLine(cfg.queueMax))
	if len(s.Tasks) == 0 {
		fmt.Println(stDim.Render("  (no active or unacknowledged tasks)"))
	}
	for _, t := range s.Tasks {
		label, st := statusStyle(t.Status)
		extra := ""
		if !t.Active() {
			extra = " — " + finalSummary(t.Dir)
		} else if t.Last.Text != "" {
			extra = " — " + shortTS(t.Last.TS) + " " + actIcon(t.Last.Kind) + " " + trunc(t.Last.Text, 100)
		}
		fmt.Printf("  %-11s %s %s%s\n", t.Label(), st.Render(fmt.Sprintf("%-19s", label)), stDim.Render(t.ID), extra)
	}
	fmt.Println(stDim.Render("  (Ctrl+C stops the view; tasks keep running)"))
}
