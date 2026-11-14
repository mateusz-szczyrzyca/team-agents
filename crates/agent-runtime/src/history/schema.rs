//! The schema of the history database: the one place that says what its tables are.
//!
//! The schema is the result of the migrations below applied in order to an empty database; `PRAGMA user_version` is the
//! number of the last one applied. A migration is never edited once it has shipped: a change of the schema is a new
//! migration (`docs/history.md`, "Versions and migrations"). `docs/history.schema.sql` pins what the migrations make.

use super::migrate::Migration;

/// The records a history made before the ledger has, as `kind, key, at, team, rank` rows: one for every row of the
/// tables of the records. `rank` breaks a tie of time between kinds, in the order a run happens in. `with_sessions` is
/// for a database of version 2 or newer, which has the sessions of runs; `without_sessions` for version 1.
///
/// The migration that makes the ledger and the view a reader makes of an older file are built from this one text, so
/// they cannot disagree about the order of a history that has no ledger.
macro_rules! ledger_records {
    (with_sessions) => {
        concat!(
            ledger_records!(without_sessions),
            " UNION ALL SELECT 'session', run_sessions.run || '/' || run_sessions.n, run_sessions.at, runs.team, 7 \
             FROM run_sessions JOIN runs ON runs.run = run_sessions.run"
        )
    };
    (without_sessions) => {
        "SELECT 'team' AS kind, team AS key, at, team, 0 AS rank FROM teams \
             UNION ALL SELECT 'round', round, at, team, 1 FROM rounds \
             UNION ALL SELECT 'run', run, at, team, 2 FROM runs \
             UNION ALL SELECT 'launch', launches.run, launches.at, runs.team, 3 \
                 FROM launches JOIN runs ON runs.run = launches.run \
             UNION ALL SELECT 'step', steps.run || '/' || steps.n, steps.at, runs.team, 4 \
                 FROM steps JOIN runs ON runs.run = steps.run \
             UNION ALL SELECT 'run_end', run_ends.run, run_ends.at, runs.team, 5 \
                 FROM run_ends JOIN runs ON runs.run = run_ends.run \
             UNION ALL SELECT 'changes', run_changes.run, COALESCE(run_ends.at, runs.at), runs.team, 6 \
                 FROM run_changes JOIN runs ON runs.run = run_changes.run \
                 LEFT JOIN run_ends ON run_ends.run = run_changes.run"
    };
}

/// The rounds each record of the ledger belongs to, as `sequence, position, round` rows, read from the ledger itself
/// and the tables of the runs: a round opening belongs to its round, and the records of a run to the rounds the run
/// works for.
macro_rules! ledger_links {
    () => {
        "SELECT sequence, 1 AS position, key AS round FROM history_records WHERE kind = 'round' \
         UNION ALL \
         SELECT history_records.sequence, run_rounds.position, run_rounds.round FROM history_records \
             JOIN run_rounds ON run_rounds.run = CASE WHEN history_records.kind IN ('step', 'session') \
                 THEN substr(history_records.key, 1, instr(history_records.key, '/') - 1) \
                 ELSE history_records.key END \
         WHERE history_records.kind IN ('run', 'launch', 'step', 'run_end', 'changes', 'session')"
    };
}

pub(super) use {ledger_links, ledger_records};

/// The first version of the database that has the sessions of runs.
pub(super) const SESSIONS_VERSION: u32 = 2;

/// The first version of the database that has the ledger.
pub(super) const LEDGER_VERSION: u32 = 3;

/// The migrations, oldest first: version 1 is the schema of item 8.4.
///
/// Every table is `STRICT`; the `CHECK`s repeat the rules of the typed records, so the database refuses what the types
/// would, and a reader that finds a row that breaks one knows the file was changed by something else. Foreign keys
/// join the rows of one record (a run and its steps); `team`, `task` and `round` in `runs` and `rounds` are plain
/// columns, so a history that starts after its team did (a deleted database, a daemon upgraded in the middle of a team)
/// records what happens from then on instead of refusing it.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
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
) STRICT;

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
) STRICT;

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
    },
    Migration {
        version: 2,
        sql: "\
CREATE TABLE run_sessions (
    run TEXT NOT NULL REFERENCES runs (run),
    n INTEGER NOT NULL CHECK (n >= 1),
    at TEXT NOT NULL,
    session TEXT NOT NULL,
    PRIMARY KEY (run, n)
) STRICT, WITHOUT ROWID;
",
    },
    Migration {
        version: 3,
        sql: concat!(
            "\
CREATE TABLE history_records (
    sequence INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN (
        'team', 'round', 'run', 'launch', 'step', 'run_end', 'changes', 'session',
        'gate', 'checked', 'pull_opened', 'pull_pushed', 'pull_ready', 'review_asked', 'review_submitted',
        'fix_pass_started', 'pull_approved', 'pull_merged', 'pull_handed_over', 'pull_closed', 'question_asked',
        'question_answered', 'controlled', 'notified', 'round_closed', 'team_ended', 'lost'
    )),
    key TEXT,
    at TEXT NOT NULL,
    team TEXT NOT NULL,
    payload TEXT,
    cut TEXT NOT NULL,
    CHECK ((kind IN ('team', 'round', 'run', 'launch', 'step', 'run_end', 'changes', 'session')) = (payload IS NULL)),
    CHECK (payload IS NULL OR (json_valid(payload) AND json_type(payload) = 'object' AND json_extract(payload, '$.event') = kind)),
    CHECK (payload IS NOT NULL OR cut = ''),
    CHECK (key IS NOT NULL OR payload IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX history_records_by_key ON history_records (kind, team, key) WHERE key IS NOT NULL;

CREATE INDEX history_records_by_team ON history_records (team, sequence);

CREATE TABLE history_record_rounds (
    sequence INTEGER NOT NULL REFERENCES history_records (sequence),
    position INTEGER NOT NULL CHECK (position >= 1),
    round TEXT NOT NULL,
    PRIMARY KEY (sequence, position),
    UNIQUE (sequence, round)
) STRICT, WITHOUT ROWID;

CREATE INDEX history_record_rounds_by_round ON history_record_rounds (round, sequence);

INSERT INTO history_records (kind, key, at, team, cut)
SELECT kind, key, at, team, '' FROM (",
            ledger_records!(with_sessions),
            ") ORDER BY at, rank, key;

INSERT INTO history_record_rounds (sequence, position, round) ",
            ledger_links!(),
            ";
"
        ),
    },
];
