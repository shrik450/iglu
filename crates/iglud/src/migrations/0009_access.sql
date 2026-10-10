-- What each workspace may do, through the channel from inside it, to itself
-- or to another of its owner's workspaces.
CREATE TABLE workspace_grant (
    holder_id     TEXT NOT NULL REFERENCES workspace (id),
    workspace_id  TEXT NOT NULL REFERENCES workspace (id),
    permission    TEXT NOT NULL CHECK (permission IN
        ('view', 'read_output', 'send_input', 'manage_columns', 'publish_routes', 'operate')),
    PRIMARY KEY (holder_id, workspace_id, permission)
) STRICT;

CREATE INDEX workspace_grant_by_target ON workspace_grant (workspace_id);

-- The workspace a person's action came through, when one did.
ALTER TABLE activity ADD COLUMN via_workspace_id TEXT REFERENCES workspace (id);
