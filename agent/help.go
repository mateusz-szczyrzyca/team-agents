package main

import (
	"fmt"
	"io"
)

// usageText is the usage help text displayed when agent is run with help, -h, --help,
// without arguments, or when an unknown command is invoked.
const usageText = `agent run <agent> "task" | -f file.md      start a task (run inside the repository)
agent status                              task list
agent agents                              configured agents and whether they are busy
agent show <id>                           task details and the last message
agent answer <id>                         open the questions in $EDITOR, validate, resume
agent resume <id> ["note"]                resume after all questions are answered
agent log <id> [--raw]                    follow the runner log (structured records rendered)
agent stop <id>                           stop a run
agent stop --running                      stop EVERYTHING that is active (teams, agents, investigations, queues)
agent clean <id>                          remove the task's worktree (the branch is kept)
agent inv [number | --all]                investigation queue / full record of one
agent dispatch                            manually start the next queued investigation (fallback)
agent live [--feed] [--remind MIN]        live panel (or an event stream with --feed)
agent web                                 web interface in the foreground (normally: agentweb.service)
agent lead                                the lead's job listener in the foreground (normally: agentlead.service; LEAD_LISTEN)
agent node                                the node daemon in the foreground (normally: agentnode.service; LEAD_URL)
agent jobs [<id>]                         remote jobs on the lead (or this node's jobs)
agent jobs --cancel <id>                  cancel a remote job on the lead
agent nodes                               registered nodes on the lead
agent kick <task>                         cut a run short: a team member sleeps (the team can wake it), solo: stopped
agent tokens <team|task>                  the tokens its runs used: per task (a team's helpers and investigations too), in all
agent tokens --runs <task>                a task's tokens per run, its steps (tool calls) and input per step: the costly runs
agent wake <task>                         resume a paused task now, not at its pause's end (a remote member's: on its node)
agent round-size <task>                   size of a task's change against the round limit (README "Rounds")
agent run --need <caps> <agent|normal|worker> "task"   submit a remote job with required capabilities
agent run --on <instance> <agent> "task"               submit a remote job pinned to an instance
agent run --resume <team|task> --same-round  continue an unmerged round (e.g. review fixes) on the same branches
agent run --last-team [--same-round] ["…"]  resume the newest team of this repository (no ID to look up)
agent ack <id|all>                        acknowledge finished tasks (they leave the panel)
agent run --team a,b[,c] [--verify|--autopilot] "task"  team; --verify: an investigator reviews the PR at the end;
                                           --autopilot: two independent reviews, the merge, the next round
agent run --investigation --team "A" "topic"     one or more investigators create and review a PR, then finish
agent autopilot <team> [on|off]            a team's --autopilot: shown, or switched (off: no merge nor next round by itself)
agent run --resume <team|task> ["addition"]  resume from the last checkpoint (same branches)
agent team [id | --all]                   teams and their members
agent team <id> --close                                close a team: stop its local members, end its remote ones' jobs
agent helpers [--all]                     worker requests made by normal models
agent transcript <id|team> [--raw]        full chronological log of a task/team (bus + activity)
agent archive <id|team> | --finished      pack a finished task (with its resume chain) into tasks-archived/*.7z
agent discard <id>...                     remove tasks that never started (worktree, branch, task directory)
agent notes [repo] [--section S] [--search Q] [-n N]   persistent repository notebook
agent authenticated [agent...]            check every agent's (or the given agents') model, with the provider's own message
agent authenticated --providers           one quick request per provider account
agent bus [N]                             bus state and the last N agent messages
agent say <agent|all|team:ID> "text"     message agents through the bus
<id> may be an unambiguous fragment of a task ID.
`

// printUsage writes the CLI usage instructions to the given writer.
func printUsage(w io.Writer) {
	fmt.Fprint(w, usageText)
}
