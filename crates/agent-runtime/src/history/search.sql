CREATE VIRTUAL TABLE history_search USING fts5(sequence UNINDEXED, log UNINDEXED, run UNINDEXED, task UNINDEXED, body, tokenize='unicode61 remove_diacritics 0');

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
FROM history_records h;
INSERT INTO history_search(rowid,log,body) SELECT -id,id,message FROM daemon_logs;

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
CREATE INDEX runs_by_agent ON runs(agent,run);
CREATE INDEX daemon_logs_by_time ON daemon_logs((substr(at,1,19)||'.'||substr(CASE WHEN substr(at,20,1)='.' THEN substr(at,21,length(at)-21) ELSE '' END||'000000000',1,9)),id);
