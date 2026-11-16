INSERT INTO expired_teams SELECT team FROM teams t
WHERE EXISTS (SELECT 1 FROM rounds r WHERE r.team = t.team AND r.round IN expired_rounds)
AND NOT EXISTS (SELECT 1 FROM rounds r WHERE r.team = t.team AND r.round NOT IN expired_rounds);

INSERT INTO expired_runs SELECT run FROM runs r
WHERE r.team IN expired_teams OR (EXISTS (SELECT 1 FROM run_rounds rr WHERE rr.run = r.run AND rr.round IN expired_rounds)
AND NOT EXISTS (SELECT 1 FROM run_rounds rr WHERE rr.run = r.run AND rr.round NOT IN expired_rounds));

INSERT INTO expired_records SELECT sequence FROM history_records h
WHERE h.team IN expired_teams OR (
 EXISTS (SELECT 1 FROM history_record_rounds hr WHERE hr.sequence = h.sequence AND hr.round IN expired_rounds)
 AND NOT EXISTS (SELECT 1 FROM history_record_rounds hr WHERE hr.sequence = h.sequence AND hr.round NOT IN expired_rounds));

INSERT INTO expired_logs SELECT id FROM daemon_logs l
WHERE l.team IN expired_teams OR (
 EXISTS (SELECT 1 FROM daemon_log_rounds lr WHERE lr.log = l.id AND lr.round IN expired_rounds)
 AND NOT EXISTS (SELECT 1 FROM daemon_log_rounds lr WHERE lr.log = l.id AND lr.round NOT IN expired_rounds));
