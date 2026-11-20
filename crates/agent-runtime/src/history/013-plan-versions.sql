CREATE TABLE plan_versions (
    id INTEGER PRIMARY KEY,
    task TEXT NOT NULL,
    team TEXT NOT NULL,
    run TEXT,
    at TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('version', 'missing', 'unreadable')),
    text TEXT,
    line INTEGER CHECK (line IS NULL OR line >= 1),
    detail TEXT,
    CHECK ((kind = 'version') = (text IS NOT NULL)),
    CHECK ((kind = 'unreadable') = (detail IS NOT NULL)),
    CHECK ((kind = 'unreadable') OR line IS NULL)
) STRICT;

CREATE INDEX plan_versions_by_task ON plan_versions (task, id);

CREATE INDEX plan_versions_by_team ON plan_versions (team, id);
