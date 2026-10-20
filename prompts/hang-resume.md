# A HUNG OPERATION WAS KILLED

There was no activity from you for {{HANG_MIN}} minutes, so the watchdog killed your last operation
(most likely a shell command that hung — e.g. it waited for input or the network, or never finished).

1. Do NOT run it again in the same form.
2. Run long or possibly hanging commands with a time limit (e.g. `timeout 600 …`) and without interactive
   prompts (`--yes`, `</dev/null`); run very long ones in the background, writing to a file, and check progress.
3. Check the state (`git status`, `read_messages`; in a team also `team_status`, `board_list`) and continue the
   task from where it stopped, following the rules from the first message of this session.
