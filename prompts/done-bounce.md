## You ended with STATUS: DONE while you hold board items: {{HELD}}

A team member's DONE takes it out of the team for good. This is a fresh session: your context is small again.

- If an item is finished, close it with `board_done` and its result.
- If you can go on with it, go on: your context is no longer a reason to stop.
- If you cannot finish it, write on it where you stopped (`board_progress`), give it back (`board_release`), and end
  with `STATUS: SLEEPING` — the coordinator will hand it on.

End with `STATUS: DONE` only when you hold no item and the coordinator has nothing more for you.
