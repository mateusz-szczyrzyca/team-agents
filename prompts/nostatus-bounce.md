## Your last turn ended without a STATUS line

So the script cannot tell whether you finished, stopped halfway, or wait for something — and nobody is there to
answer (`--autopilot`). **Continue as planned.**

- Run every command in the foreground and wait for its result in the same turn. Never end a turn while a command —
  a gate, a test run, a build — still runs in the background: ending the turn stops it, and its result is lost.
- End every turn with exactly one STATUS line, as your instructions say.
