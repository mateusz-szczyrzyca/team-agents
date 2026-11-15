CREATE TABLE history_cursors (
    name TEXT PRIMARY KEY CHECK (name = 'daemon_logs'),
    value INTEGER NOT NULL CHECK (value >= 0)
) STRICT;
INSERT INTO history_cursors SELECT 'daemon_logs', coalesce(max(id), 0) FROM daemon_logs;
CREATE TRIGGER daemon_logs_cursor AFTER INSERT ON daemon_logs BEGIN
 UPDATE history_cursors SET value = max(value, new.id) WHERE name = 'daemon_logs';
END;
