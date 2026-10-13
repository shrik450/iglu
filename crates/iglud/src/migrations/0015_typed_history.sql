-- A workspace's history and its conditions are data now, which whoever
-- shows them says in words, not English iglud wrote. What an entry needed to
-- be said was only in its English, so only the entries that need nothing
-- more are kept. A failing step's condition comes back on the reconciler's
-- next try.
CREATE TABLE activity_typed (
    id                INTEGER PRIMARY KEY,
    workspace_id      TEXT REFERENCES workspace (id),
    actor_id          TEXT,
    via_workspace_id  TEXT REFERENCES workspace (id),
    what              TEXT NOT NULL,
    at                INTEGER NOT NULL
) STRICT;

INSERT INTO activity_typed (id, workspace_id, actor_id, via_workspace_id, what, at)
SELECT id, workspace_id, actor_id, via_workspace_id, json_object('kind', CASE kind
    WHEN 'created' THEN 'Created'
    WHEN 'started' THEN 'Started'
    WHEN 'secrets-delivered' THEN 'SecretsDelivered'
    WHEN 'provisioned' THEN 'Provisioned'
    WHEN 'columns-opened' THEN 'ColumnsOpened'
    WHEN 'frozen' THEN 'Frozen'
    WHEN 'thawed' THEN 'Thawed'
    WHEN 'stopped' THEN 'Stopped'
    WHEN 'removed' THEN 'Removed'
    WHEN 'deleted' THEN 'Deleted'
    WHEN 'thawed-on-open' THEN 'ThawedOnOpen'
    WHEN 'idle-froze' THEN 'IdleFroze'
    WHEN 'idle-stopped' THEN 'IdleStopped'
    WHEN 'access-changed' THEN 'AccessChanged'
    WHEN 'updated' THEN 'Updated'
END), at
FROM activity
WHERE kind IN (
    'created', 'started', 'secrets-delivered', 'provisioned', 'columns-opened',
    'frozen', 'thawed', 'stopped', 'removed', 'deleted', 'thawed-on-open',
    'idle-froze', 'idle-stopped', 'access-changed', 'updated'
);

DROP TABLE activity;
ALTER TABLE activity_typed RENAME TO activity;
CREATE INDEX activity_by_workspace ON activity (workspace_id, at);

UPDATE workspace SET condition = NULL WHERE json_extract(condition, '$.kind') = 'error';
