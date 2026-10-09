-- Projects: what a person works on and how its workspaces start. Every
-- workspace belongs to one. Each person's built-in project has no
-- repository and can't be deleted.
CREATE TABLE project (
    id              TEXT PRIMARY KEY,
    owner_id        TEXT NOT NULL REFERENCES principal (id),
    name            TEXT NOT NULL,
    origin          TEXT NOT NULL CHECK (origin IN ('builtin', 'added')),
    repo            TEXT,
    environment_id  TEXT NOT NULL REFERENCES environment (id),
    -- The columns its workspaces open with, as JSON.
    opening         TEXT NOT NULL,
    -- The agent its workspaces start, if any.
    agent           TEXT,
    -- Ports its workspaces publish when created, as JSON.
    ports           TEXT NOT NULL DEFAULT '[]',
    -- How its workspaces idle, as JSON.
    idle            TEXT NOT NULL DEFAULT '{"kind":"default"}',
    revision        INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    UNIQUE (owner_id, name)
) STRICT;

CREATE UNIQUE INDEX project_builtin ON project (owner_id) WHERE origin = 'builtin';

-- What a workspace cloned, for workspaces in projects with a repository. Kept
-- as created: provisioning retries need it even if the project changes.
CREATE TABLE workspace_checkout (
    workspace_id  TEXT PRIMARY KEY REFERENCES workspace (id),
    repo          TEXT NOT NULL,
    branch        TEXT NOT NULL,
    base          TEXT
) STRICT;

INSERT INTO workspace_checkout (workspace_id, repo, branch, base)
    SELECT id, repo, branch, base FROM workspace;

ALTER TABLE workspace DROP COLUMN repo;
ALTER TABLE workspace DROP COLUMN branch;
ALTER TABLE workspace DROP COLUMN base;

-- Filled for existing workspaces by the next migration, and on every insert.
-- SQLite can't add a NOT NULL reference, so reads treat NULL as corrupt.
ALTER TABLE workspace ADD COLUMN project_id TEXT REFERENCES project (id);
