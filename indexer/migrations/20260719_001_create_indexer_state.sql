-- Tracks the last block the indexer has fully processed, so a restart
-- can resume backfilling from there instead of re-scanning from the
-- contract's deployment block every time. Single-row table (id=1) since
-- the indexer currently tracks exactly one contract on one chain.
CREATE TABLE IF NOT EXISTS indexer_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_indexed_block INTEGER NOT NULL
);
