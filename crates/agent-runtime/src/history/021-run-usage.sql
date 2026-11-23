-- Version 21: a run's context size, output tokens and cost as its stream told them, kept in `run_usage` outside the
-- ledger, like judgements. A history made before has no row for its runs: they read as "not told".
PRAGMA defer_foreign_keys = ON;
CREATE TABLE run_usage (
    run TEXT NOT NULL PRIMARY KEY REFERENCES runs (run),
    first_context INTEGER CHECK (first_context IS NULL OR first_context >= 0),
    last_context INTEGER CHECK (last_context IS NULL OR last_context >= 0),
    tokens_written INTEGER CHECK (tokens_written IS NULL OR tokens_written >= 0),
    cost_usd REAL CHECK (cost_usd IS NULL OR (cost_usd >= 0 AND cost_usd < 1e18))
) STRICT, WITHOUT ROWID;
