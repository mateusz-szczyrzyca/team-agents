-- Transaction-local sets keep readers on a complete snapshot.
CREATE TEMP TABLE expired_rounds (round TEXT PRIMARY KEY) STRICT;
CREATE TEMP TABLE expired_teams (team TEXT PRIMARY KEY) STRICT;
CREATE TEMP TABLE expired_runs (run TEXT PRIMARY KEY) STRICT;
CREATE TEMP TABLE expired_records (sequence INTEGER PRIMARY KEY) STRICT;
CREATE TEMP TABLE expired_logs (id INTEGER PRIMARY KEY) STRICT;
