-- Attention per thread: one conversation of a session's agent, or the
-- session itself. Statuses are reported again within seconds, so the old
-- ones aren't carried over.
DROP TABLE attention;

CREATE TABLE attention (
    workspace_id  TEXT NOT NULL REFERENCES workspace (id),
    session       TEXT NOT NULL,
    thread        TEXT NOT NULL,
    -- What the thread was started to do, or empty.
    title         TEXT NOT NULL,
    state         TEXT NOT NULL,
    summary       TEXT NOT NULL,
    updated_at    INTEGER NOT NULL,
    seen_at       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, session, thread)
) STRICT;
