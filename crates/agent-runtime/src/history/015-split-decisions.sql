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
