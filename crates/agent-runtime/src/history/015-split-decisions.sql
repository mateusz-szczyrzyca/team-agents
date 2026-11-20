-- Preserve records, round links and search while recording split proposal decisions.
PRAGMA defer_foreign_keys = ON;
CREATE TEMP TABLE saved_finding_history AS SELECT * FROM history_records;
DROP TABLE history_records;
CREATE TABLE history_records (
    sequence INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN (
        'split_decision', 'team', 'round', 'run', 'launch', 'step', 'run_end', 'changes', 'session',
        'gate', 'checked', 'pull_opened', 'pull_pushed', 'pull_ready', 'review_asked', 'review_submitted',
        'finding_explained', 'finding_accepted', 'finding_rejected', 'finding_raised',
        'second_opinion_asked', 'second_opinion_ended', 'finding_confirmed', 'finding_not_confirmed',
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

INSERT INTO history_records SELECT * FROM saved_finding_history;
DROP TABLE saved_finding_history;
CREATE UNIQUE INDEX history_records_by_key ON history_records (kind, team, key) WHERE key IS NOT NULL;

CREATE INDEX history_records_by_team ON history_records (team, sequence);

CREATE TRIGGER history_search_insert AFTER INSERT ON history_records BEGIN
 INSERT INTO history_search(rowid,sequence,run,task,body) SELECT h.sequence,h.sequence,
CASE WHEN h.kind IN ('run','launch','run_end','changes') THEN h.key
     WHEN h.kind IN ('step','session') THEN substr(h.key,1,instr(h.key,'/')-1) END AS run,
json_extract(h.payload,'$.task') AS task,
coalesce(h.key,'') || ' ' || coalesce((SELECT group_concat(value,' ') FROM json_tree(h.payload) WHERE type='text'),'') || ' ' || coalesce(CASE h.kind
 WHEN 'run' THEN (SELECT prompt FROM runs WHERE run=h.key)
 WHEN 'step' THEN (SELECT coalesce(text,'') || ' ' || coalesce(detail,'') || ' ' || coalesce(path,'') || ' ' || coalesce(tool,'') || ' ' || coalesce(action,'') || ' ' || coalesce(login,'') FROM steps WHERE run=substr(h.key,1,instr(h.key,'/')-1) AND n=CAST(substr(h.key,instr(h.key,'/')+1) AS INTEGER))
 WHEN 'run_end' THEN (SELECT final_message FROM run_ends WHERE run=h.key)
 WHEN 'team' THEN (SELECT task_list FROM teams WHERE team=h.key)
 WHEN 'round' THEN (SELECT coalesce(brief,'') || ' ' || coalesce(title,'') || ' ' || coalesce(description,'') FROM rounds WHERE round=h.key)
END,'') AS body
FROM history_records h WHERE sequence=new.sequence;
END;
CREATE TRIGGER history_search_delete AFTER DELETE ON history_records BEGIN
 DELETE FROM history_search WHERE rowid=old.sequence;
END;
CREATE TRIGGER history_search_update AFTER UPDATE ON history_records BEGIN
 DELETE FROM history_search WHERE rowid=old.sequence;
 INSERT INTO history_search(rowid,sequence,run,task,body) SELECT h.sequence,h.sequence,
CASE WHEN h.kind IN ('run','launch','run_end','changes') THEN h.key
     WHEN h.kind IN ('step','session') THEN substr(h.key,1,instr(h.key,'/')-1) END AS run,
json_extract(h.payload,'$.task') AS task,
coalesce(h.key,'') || ' ' || coalesce((SELECT group_concat(value,' ') FROM json_tree(h.payload) WHERE type='text'),'') || ' ' || coalesce(CASE h.kind
 WHEN 'run' THEN (SELECT prompt FROM runs WHERE run=h.key)
 WHEN 'step' THEN (SELECT coalesce(text,'') || ' ' || coalesce(detail,'') || ' ' || coalesce(path,'') || ' ' || coalesce(tool,'') || ' ' || coalesce(action,'') || ' ' || coalesce(login,'') FROM steps WHERE run=substr(h.key,1,instr(h.key,'/')-1) AND n=CAST(substr(h.key,instr(h.key,'/')+1) AS INTEGER))
 WHEN 'run_end' THEN (SELECT final_message FROM run_ends WHERE run=h.key)
 WHEN 'team' THEN (SELECT task_list FROM teams WHERE team=h.key)
 WHEN 'round' THEN (SELECT coalesce(brief,'') || ' ' || coalesce(title,'') || ' ' || coalesce(description,'') FROM rounds WHERE round=h.key)
END,'') AS body
FROM history_records h WHERE sequence=new.sequence;
END;

-- Preserve existing logs and their links while admitting split proposal decisions.
PRAGMA defer_foreign_keys = ON;
CREATE TEMP TABLE saved_daemon_logs AS SELECT * FROM daemon_logs;
DROP TABLE daemon_logs;
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
        'request_refused', 'daemon_panicked', 'daemon_task_failed', 'team_not_started', 'notification_failed',
        'history_record_incomplete', 'team_changed', 'round_changed', 'item_phase_changed', 'task_status_changed',
        'run_started', 'run_ended', 'daemon_started', 'daemon_stop_requested', 'daemon_stopped', 'request_delegated',
        'questions_answered', 'control_used', 'checks_decided', 'reviews_requested', 'review_report_late',
        'notification_sent', 'split_decision', 'questions_file_opened', 'build_directory_removed', 'decision_taken',
        'outside_call_completed', 'peer_gone', 'github_step_waiting', 'github_step_resumed', 'snapshot_failed', 'snapshot_large', 'delivery_closed'
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
        (level = 'error' AND event IN (
            'state_save_failed', 'configuration_name_missing', 'daemon_panicked', 'daemon_task_failed',
            'team_not_started', 'notification_failed'
        ))
        OR (level = 'warn' AND event IN (
            'cli_output_unreadable', 'run_ended_without_status', 'run_hung', 'run_quota_exhausted', 'run_rate_limited',
            'bus_down', 'git_call_failed', 'github_call_failed', 'gate_cannot_start', 'gate_timed_out',
            'gate_dirty_tree', 'questions_file_invalid', 'restart_corrected', 'request_refused',
            'history_record_incomplete', 'github_step_waiting', 'snapshot_failed', 'snapshot_large', 'delivery_closed'
        ))
        OR (level = 'info' AND event IN (
            'team_changed', 'round_changed', 'item_phase_changed', 'task_status_changed', 'run_started', 'run_ended',
            'daemon_started', 'daemon_stop_requested', 'daemon_stopped', 'request_delegated', 'questions_answered',
            'control_used', 'checks_decided', 'reviews_requested', 'review_report_late', 'notification_sent',
            'split_decision', 'questions_file_opened', 'build_directory_removed', 'github_step_resumed'
        ))
        OR (level = 'debug' AND event IN ('decision_taken', 'outside_call_completed', 'peer_gone'))
    )
) STRICT;
INSERT INTO daemon_logs SELECT * FROM saved_daemon_logs;
DROP TABLE saved_daemon_logs;
CREATE INDEX daemon_logs_by_team ON daemon_logs (team, id) WHERE team IS NOT NULL;
CREATE INDEX daemon_logs_by_task ON daemon_logs (task, id) WHERE task IS NOT NULL;
CREATE INDEX daemon_logs_by_run ON daemon_logs (run, id) WHERE run IS NOT NULL;
CREATE INDEX daemon_logs_by_level ON daemon_logs (level, id);
CREATE INDEX daemon_logs_warn_coalescing ON daemon_logs (event, team, task, run, message, fields, cut)
    WHERE level = 'warn';
CREATE TRIGGER log_search_insert AFTER INSERT ON daemon_logs BEGIN
 INSERT INTO history_search(rowid,log,body) VALUES(-new.id,new.id,new.message);
END;
CREATE TRIGGER log_search_delete AFTER DELETE ON daemon_logs BEGIN
 DELETE FROM history_search WHERE rowid=-old.id;
END;
CREATE TRIGGER log_search_update AFTER UPDATE ON daemon_logs BEGIN
 DELETE FROM history_search WHERE rowid=-old.id;
 INSERT INTO history_search(rowid,log,body) VALUES(-new.id,new.id,new.message);
END;
CREATE INDEX daemon_logs_by_event ON daemon_logs(event,id);
CREATE INDEX daemon_logs_by_time ON daemon_logs((substr(at,1,19)||'.'||substr(CASE WHEN substr(at,20,1)='.' THEN substr(at,21,length(at)-21) ELSE '' END||'000000000',1,9)),id);
CREATE TRIGGER daemon_logs_cursor AFTER INSERT ON daemon_logs BEGIN
 UPDATE history_cursors SET value = max(value, new.id) WHERE name = 'daemon_logs';
END;
