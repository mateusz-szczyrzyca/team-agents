-- The schema of history.db at user_version 4: what `sqlite_master` holds after the migrations of
-- crates/agent-runtime/src/history/schema.rs, in the order they create it. A test compares it with a fresh database.
PRAGMA user_version = 4;

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

CREATE TABLE run_sessions (
    run TEXT NOT NULL REFERENCES runs (run),
    n INTEGER NOT NULL CHECK (n >= 1),
    at TEXT NOT NULL,
    session TEXT NOT NULL,
    PRIMARY KEY (run, n)
) STRICT, WITHOUT ROWID;

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

CREATE TABLE daemon_logs (
    id INTEGER PRIMARY KEY,
    first_at TEXT NOT NULL,
    at TEXT NOT NULL,
    level TEXT NOT NULL CHECK (level IN ('error', 'warn', 'info', 'debug')),
    module TEXT NOT NULL CHECK (module <> ''),
    event TEXT NOT NULL CHECK (event IN (
        'cli_output_unreadable', 'run_ended_without_status', 'run_hung', 'run_quota_exhausted', 'run_rate_limited',
        'bus_down', 'git_call_failed', 'github_call_failed', 'gate_cannot_start', 'gate_timed_out', 'gate_dirty_tree',
        'state_save_failed', 'questions_file_invalid', 'configuration_name_missing', 'restart_corrected',
        'request_refused', 'daemon_panicked', 'team_changed', 'round_changed', 'item_phase_changed',
        'task_status_changed', 'run_started', 'run_ended', 'decision_taken', 'outside_call_completed'
    )),
    team TEXT,
    task TEXT,
    run TEXT,
    message TEXT NOT NULL,
    fields TEXT NOT NULL CHECK (json_valid(fields) AND json_type(fields) = 'object'),
    count INTEGER NOT NULL CHECK (count >= 1 AND (level = 'warn' OR count = 1)),
    cut TEXT NOT NULL,
    CHECK (first_at <= at),
    CHECK (run IS NULL OR task IS NOT NULL),
    CHECK (
        (level = 'error' AND event IN ('state_save_failed', 'configuration_name_missing', 'daemon_panicked'))
        OR (level = 'warn' AND event IN (
            'cli_output_unreadable', 'run_ended_without_status', 'run_hung', 'run_quota_exhausted', 'run_rate_limited',
            'bus_down', 'git_call_failed', 'github_call_failed', 'gate_cannot_start', 'gate_timed_out',
            'gate_dirty_tree', 'questions_file_invalid', 'restart_corrected', 'request_refused'
        ))
        OR (level = 'info' AND event IN (
            'team_changed', 'round_changed', 'item_phase_changed', 'task_status_changed', 'run_started', 'run_ended'
        ))
        OR (level = 'debug' AND event IN ('decision_taken', 'outside_call_completed'))
    )
) STRICT;

CREATE INDEX daemon_logs_by_team ON daemon_logs (team, id) WHERE team IS NOT NULL;

CREATE INDEX daemon_logs_by_task ON daemon_logs (task, id) WHERE task IS NOT NULL;

CREATE INDEX daemon_logs_by_run ON daemon_logs (run, id) WHERE run IS NOT NULL;

CREATE INDEX daemon_logs_by_level ON daemon_logs (level, id);

CREATE INDEX daemon_logs_warn_coalescing ON daemon_logs (event, team, task, run, message, fields, cut)
    WHERE level = 'warn';

CREATE TABLE daemon_log_rounds (
    log INTEGER NOT NULL REFERENCES daemon_logs (id),
    position INTEGER NOT NULL CHECK (position >= 1),
    round TEXT NOT NULL,
    PRIMARY KEY (log, position),
    UNIQUE (log, round)
) STRICT, WITHOUT ROWID;

CREATE INDEX daemon_log_rounds_by_round ON daemon_log_rounds (round, log);
