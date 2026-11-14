//! The schema of the history database: the one place that says what its tables are.
//!
//! The schema is the result of the migrations below applied in order to an empty database; `PRAGMA user_version` is the
//! number of the last one applied. A migration is never edited once it has shipped: a change of the schema is a new
//! migration (`docs/history.md`, "Versions and migrations"). `docs/history.schema.sql` pins what the migrations make.

use super::migrate::Migration;

/// The migrations, oldest first: version 1 is the schema of item 8.4.
///
/// Every table is `STRICT`; the `CHECK`s repeat the rules of the typed records, so the database refuses what the types
/// would, and a reader that finds a row that breaks one knows the file was changed by something else. Foreign keys
/// join the rows of one record (a run and its steps); `team`, `task` and `round` in `runs` and `rounds` are plain
/// columns, so a history that starts after its team did (a deleted database, a daemon upgraded in the middle of a team)
/// records what happens from then on instead of refusing it.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: "\
CREATE TABLE teams (
    team TEXT NOT NULL PRIMARY KEY,
    at TEXT NOT NULL,
    repo TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    merge_mode TEXT NOT NULL CHECK (merge_mode IN ('autopilot', 'no_merge')),
    task_list TEXT NOT NULL,
    cut TEXT NOT NULL
) STRICT;

CREATE TABLE plan (
    team TEXT NOT NULL REFERENCES teams (team),
    position INTEGER NOT NULL CHECK (position >= 1),
    agent TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('solo', 'coordinator', 'member', 'worker')),
    how TEXT NOT NULL CHECK (how IN ('named', 'named_by_flag', 'named_by_line', 'drawn', 'configured_order')),
    drawn_among TEXT CHECK (drawn_among IS NULL OR (json_valid(drawn_among) AND json_type(drawn_among) = 'array')),
    CHECK ((how = 'drawn') = (drawn_among IS NOT NULL)),
    PRIMARY KEY (team, position)
) STRICT, WITHOUT ROWID;

CREATE TABLE rounds (
    round TEXT NOT NULL PRIMARY KEY,
    team TEXT NOT NULL,
    at TEXT NOT NULL,
    item TEXT,
    title TEXT,
    description TEXT,
    expected_result TEXT,
    checks TEXT,
    brief TEXT,
    cut TEXT NOT NULL,
    CHECK (
        (item IS NOT NULL AND title IS NOT NULL AND description IS NOT NULL AND brief IS NULL)
        OR (item IS NULL AND title IS NULL AND description IS NULL AND expected_result IS NULL
            AND checks IS NULL AND brief IS NOT NULL)
    )
) STRICT, WITHOUT ROWID;

CREATE INDEX rounds_by_team ON rounds (team, at);

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
        'answers', 'resumed_by_human', 'continuation'
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
) STRICT, WITHOUT ROWID;

CREATE INDEX runs_by_team ON runs (team, at);
CREATE INDEX runs_by_task ON runs (task, at);

CREATE TABLE run_rounds (
    run TEXT NOT NULL REFERENCES runs (run),
    position INTEGER NOT NULL CHECK (position >= 1),
    round TEXT NOT NULL,
    PRIMARY KEY (run, position),
    UNIQUE (run, round)
) STRICT, WITHOUT ROWID;

CREATE INDEX run_rounds_by_round ON run_rounds (round, run);

CREATE TABLE launches (
    run TEXT NOT NULL PRIMARY KEY REFERENCES runs (run),
    at TEXT NOT NULL,
    program TEXT NOT NULL,
    args TEXT NOT NULL CHECK (json_valid(args) AND json_type(args) = 'array'),
    working_dir TEXT NOT NULL,
    head TEXT,
    env_names TEXT NOT NULL CHECK (json_valid(env_names) AND json_type(env_names) = 'array'),
    prompt_in_file INTEGER NOT NULL CHECK (prompt_in_file IN (0, 1)),
    cut TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE steps (
    run TEXT NOT NULL REFERENCES runs (run),
    n INTEGER NOT NULL CHECK (n >= 1),
    at TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN (
        'message', 'command', 'file_edit', 'bus_call', 'tool_call', 'error', 'info', 'control'
    )),
    text TEXT,
    path TEXT,
    tool TEXT,
    detail TEXT,
    action TEXT CHECK (action IS NULL OR action IN ('stop', 'pause', 'unpause', 'resume', 'say')),
    client TEXT CHECK (client IS NULL OR client IN ('cli', 'dashboard')),
    login TEXT,
    cut TEXT NOT NULL,
    CHECK (
        (kind IN ('message', 'command', 'error', 'info') AND text IS NOT NULL AND path IS NULL AND tool IS NULL
            AND detail IS NULL AND action IS NULL AND client IS NULL AND login IS NULL)
        OR (kind = 'file_edit' AND path IS NOT NULL AND text IS NULL AND tool IS NULL AND detail IS NULL
            AND action IS NULL AND client IS NULL AND login IS NULL)
        OR (kind IN ('bus_call', 'tool_call') AND tool IS NOT NULL AND detail IS NOT NULL AND text IS NULL
            AND path IS NULL AND action IS NULL AND client IS NULL AND login IS NULL)
        OR (kind = 'control' AND action IS NOT NULL AND client IS NOT NULL AND text IS NULL AND path IS NULL
            AND tool IS NULL AND detail IS NULL AND ((client = 'dashboard') = (login IS NOT NULL)))
    ),
    PRIMARY KEY (run, n)
) STRICT, WITHOUT ROWID;

CREATE TABLE run_ends (
    run TEXT NOT NULL PRIMARY KEY REFERENCES runs (run),
    at TEXT NOT NULL,
    how TEXT NOT NULL CHECK (how IN ('reported', 'no_status', 'quota_exhausted', 'rate_limited', 'hung', 'stopped')),
    outcome TEXT CHECK (outcome IS NULL OR outcome IN ('DONE', 'QUESTIONS', 'FAILED', 'SLEEPING', 'EXCLUDED')),
    exit_code INTEGER,
    resets_at TEXT,
    final_message TEXT,
    cut TEXT NOT NULL,
    CHECK ((how = 'reported') = (outcome IS NOT NULL)),
    CHECK (exit_code IS NULL OR how = 'no_status'),
    CHECK (resets_at IS NULL OR how = 'quota_exhausted')
) STRICT, WITHOUT ROWID;

CREATE TABLE run_changes (
    run TEXT NOT NULL PRIMARY KEY REFERENCES runs (run),
    at TEXT NOT NULL,
    head_before TEXT,
    head_after TEXT
) STRICT, WITHOUT ROWID;

CREATE TABLE commits (
    run TEXT NOT NULL REFERENCES run_changes (run),
    n INTEGER NOT NULL CHECK (n >= 1),
    hash TEXT NOT NULL,
    subject TEXT NOT NULL,
    authored_at TEXT NOT NULL,
    cut TEXT NOT NULL,
    PRIMARY KEY (run, n)
) STRICT, WITHOUT ROWID;

CREATE TABLE changed_files (
    run TEXT NOT NULL REFERENCES run_changes (run),
    scope TEXT NOT NULL CHECK (scope IN ('committed', 'uncommitted')),
    path TEXT NOT NULL,
    change TEXT NOT NULL CHECK (change IN ('added', 'deleted', 'modified', 'renamed')),
    renamed_from TEXT,
    is_binary INTEGER NOT NULL CHECK (is_binary IN (0, 1)),
    lines_added INTEGER,
    lines_removed INTEGER,
    CHECK ((change = 'renamed') = (renamed_from IS NOT NULL)),
    CHECK (
        (is_binary = 1 AND lines_added IS NULL AND lines_removed IS NULL)
        OR (is_binary = 0 AND lines_added >= 0 AND lines_removed >= 0)
    ),
    PRIMARY KEY (run, scope, path)
) STRICT, WITHOUT ROWID;
",
}];
