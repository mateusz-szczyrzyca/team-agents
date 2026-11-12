# A HUNG OPERATION WAS KILLED

There was no activity from you for {{minutes}} minutes, so the watchdog killed your last operation — most likely a
shell command that hung: it waited for input or the network, or never finished. This is the same session.

1. Do not run it again in the same form.
2. Run long or possibly hanging commands with a time limit (`timeout 600 …`) and without interactive prompts
   (`--yes`, `</dev/null`).
3. Check where you are (`git status`, `git log -3`) and continue the task from where it stopped, following the rules
   of the first message of this session.
4. End your turn with a `STATUS:` line, as before.
