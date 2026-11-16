-- Rebuild the two closed keyword sets without changing shipped migrations.
-- Defer references while the original table names are temporarily absent.
PRAGMA defer_foreign_keys = ON;

CREATE TEMP TABLE saved_runs AS SELECT * FROM runs;
DROP TABLE runs;
CREATE TABLE runs (
    run TEXT NOT NULL PRIMARY KEY,
    team TEXT NOT NULL,
    task TEXT NOT NULL,
    at TEXT NOT NULL,
    agent TEXT NOT NULL,
    tier TEXT NOT NULL CHECK (tier IN ('normal', 'worker', 'investigator')),
    cli TEXT NOT NULL CHECK (cli IN ('claude', 'codex', 'gemini', 'opencode')),
    model TEXT,
    role TEXT NOT NULL CHECK (role IN ('solo', 'coordinator', 'member', 'worker', 'reviewer')),
    why TEXT NOT NULL CHECK (why IN (
        'first_run', 'item', 'wake_up', 'request', 'fix_pass', 'review', 'ask_human', 'verdict_missing', 'checks',
        'answers', 'resumed_by_human', 'continuation', 'turn_limit'
    )),
    why_item TEXT,
    why_asked_by TEXT,
    why_pass INTEGER,
    why_pr INTEGER,
    prompt TEXT NOT NULL,
    worktree TEXT NOT NULL,
    branch TEXT NOT NULL,
    session TEXT,
    cut TEXT NOT NULL,
    CHECK (
        (why = 'item') = (why_item IS NOT NULL)
        AND (why = 'request' OR why_asked_by IS NULL)
        AND ((why IN ('fix_pass', 'review')) = (why_pass IS NOT NULL))
        AND ((why = 'review') = (why_pr IS NOT NULL))
        AND (why <> 'fix_pass' OR why_pass >= 1)
        AND (why_pass IS NULL OR why_pass >= 0)
        AND (why_pr IS NULL OR why_pr >= 1)
    )
) STRICT;
INSERT INTO runs SELECT * FROM saved_runs;
DROP TABLE saved_runs;

CREATE TEMP TABLE saved_run_ends AS SELECT * FROM run_ends;
DROP TABLE run_ends;
CREATE TABLE run_ends (
    run TEXT NOT NULL PRIMARY KEY REFERENCES runs (run),
    at TEXT NOT NULL,
    how TEXT NOT NULL CHECK (how IN ('reported', 'no_status', 'quota_exhausted', 'rate_limited', 'hung', 'stopped', 'turn_limit')),
    outcome TEXT CHECK (outcome IS NULL OR outcome IN ('DONE', 'QUESTIONS', 'FAILED', 'SLEEPING', 'EXCLUDED')),
    exit_code INTEGER,
    resets_at TEXT,
    final_message TEXT,
    cut TEXT NOT NULL,
    CHECK ((how = 'reported') = (outcome IS NOT NULL)),
    CHECK (exit_code IS NULL OR how = 'no_status'),
    CHECK (resets_at IS NULL OR how = 'quota_exhausted')
) STRICT, WITHOUT ROWID;
INSERT INTO run_ends SELECT * FROM saved_run_ends;
DROP TABLE saved_run_ends;

CREATE INDEX runs_by_team ON runs (team, at);
CREATE INDEX runs_by_task ON runs (task, at);
CREATE INDEX runs_by_agent ON runs(agent,run);
