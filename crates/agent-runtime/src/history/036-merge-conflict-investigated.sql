-- Admit automatic conflict investigations while preserving log links, search and cursors.
PRAGMA defer_foreign_keys = ON;
CREATE TEMP TABLE saved_logs AS SELECT * FROM daemon_logs;
CREATE TEMP TABLE saved_log_rounds AS SELECT * FROM daemon_log_rounds;
DELETE FROM history_search WHERE log IS NOT NULL;
DELETE FROM daemon_log_rounds;
DROP TABLE daemon_logs;
CREATE TABLE daemon_logs (
    id INTEGER PRIMARY KEY,
    first_at TEXT NOT NULL,
    at TEXT NOT NULL,
    level TEXT NOT NULL CHECK (level IN ('error', 'warn', 'info', 'debug')),
    module TEXT NOT NULL CHECK (module <> ''),
    event TEXT NOT NULL CHECK (event IN (
        'cli_output_unreadable', 'run_ended_without_status', 'run_no_tool_failed', 'run_hung', 'run_quota_exhausted', 'run_rate_limited',
        'bus_down', 'git_call_failed', 'github_call_failed', 'gate_cannot_start', 'gate_timed_out', 'gate_dirty_tree',
        'state_save_failed', 'questions_file_invalid', 'configuration_name_missing', 'restart_corrected',
        'request_refused', 'daemon_panicked', 'daemon_task_failed', 'team_not_started', 'notification_failed',
        'history_record_incomplete', 'team_changed', 'team_removed', 'round_changed', 'item_phase_changed', 'task_status_changed',
        'run_started', 'run_ended', 'daemon_started', 'daemon_stop_requested', 'daemon_stopped', 'request_delegated',
        'questions_answered', 'control_used', 'serena_reminded', 'checks_decided', 'reviews_requested', 'review_report_late',
        'notification_sent', 'questions_file_opened', 'build_directory_removed', 'decision_taken',
        'outside_call_completed', 'peer_gone', 'github_step_waiting', 'github_step_resumed', 'snapshot_failed', 'snapshot_large', 'delivery_closed',
        'task_handed_over', 'task_waits_for_agent', 'worker_request_damaged', 'run_no_tool_retry',
        'commit_reminded', 'commit_followed', 'context_reminded', 'commit_cut', 'worker_request_returned', 'worker_request_requeued', 'worker_wait_returned', 'worker_request_ended', 'task_agent_changed', 'task_wait_for_free_agent', 'worktree_switched', 'worktree_switch_refused', 'publish_off_branch', 'checkpoint_failed', 'investigation_started', 'investigation_answered', 'investigation_requeued', 'investigation_failed', 'merge_conflict_investigated', 'investigation_kind_unknown'
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
            'cli_output_unreadable', 'run_ended_without_status', 'run_no_tool_failed', 'run_hung', 'run_quota_exhausted', 'run_rate_limited',
            'bus_down', 'git_call_failed', 'github_call_failed', 'gate_cannot_start', 'gate_timed_out',
            'gate_dirty_tree', 'questions_file_invalid', 'restart_corrected', 'request_refused',
            'history_record_incomplete', 'github_step_waiting', 'snapshot_failed', 'snapshot_large', 'delivery_closed',
            'worker_request_damaged', 'worktree_switch_refused', 'publish_off_branch', 'checkpoint_failed', 'investigation_kind_unknown'
        ))
        OR (level = 'info' AND event IN (
            'team_changed', 'team_removed', 'round_changed', 'item_phase_changed', 'task_status_changed', 'run_started', 'run_ended',
            'daemon_started', 'daemon_stop_requested', 'daemon_stopped', 'request_delegated', 'questions_answered',
            'control_used', 'serena_reminded', 'checks_decided', 'reviews_requested', 'review_report_late', 'notification_sent',
            'questions_file_opened', 'build_directory_removed', 'github_step_resumed', 'task_handed_over',
            'task_waits_for_agent', 'run_no_tool_retry', 'commit_reminded', 'commit_followed', 'context_reminded', 'commit_cut', 'worker_request_returned', 'worker_request_requeued', 'worker_wait_returned', 'worker_request_ended', 'task_agent_changed', 'task_wait_for_free_agent', 'worktree_switched', 'investigation_started', 'investigation_answered', 'investigation_requeued', 'investigation_failed', 'merge_conflict_investigated'
        ))
        OR (level = 'debug' AND event IN ('decision_taken', 'outside_call_completed', 'peer_gone'))
    )
) STRICT;
INSERT INTO daemon_logs SELECT * FROM saved_logs;
DROP TABLE saved_logs;
INSERT INTO daemon_log_rounds SELECT * FROM saved_log_rounds;
DROP TABLE saved_log_rounds;
INSERT INTO history_search(rowid,log,body) SELECT -id,id,message FROM daemon_logs;
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
