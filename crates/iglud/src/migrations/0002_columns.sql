-- A workspace's columns: the terminal sessions it opens each boot, what each
-- runs, and how wide each shows. Intent: the guest's sessions are what exists.
CREATE TABLE workspace_column (
    workspace_id  TEXT NOT NULL REFERENCES workspace (id),
    position      INTEGER NOT NULL,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,
    width         TEXT NOT NULL,
    -- The prompt the column's agent starts with, until the first opening.
    prompt        TEXT,
    PRIMARY KEY (workspace_id, name)
) STRICT;

INSERT INTO workspace_column (workspace_id, position, name, kind, width)
    SELECT id, 0, 'shell', '{"kind":"shell"}', 'half' FROM workspace WHERE deleted_at IS NULL;
