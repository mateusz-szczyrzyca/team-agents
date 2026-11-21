CREATE TABLE step_judgements (
    id INTEGER PRIMARY KEY,
    task TEXT NOT NULL,
    team TEXT NOT NULL,
    run TEXT,
    at TEXT NOT NULL,
    step TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('confirmed', 'declared', 'mismatch')),
    check_line TEXT,
    reason TEXT,
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    output_path TEXT
) STRICT;

CREATE INDEX step_judgements_by_task ON step_judgements (task, id);

CREATE INDEX step_judgements_by_team ON step_judgements (team, id);
