-- What's stored as JSON names its enums' variants as the Rust code does, so
-- one name finds a variant in Rust, in the console and in the database.
-- project.idle's default, from 0003, still has the old spelling: iglud
-- always writes the rule itself.
UPDATE workspace SET condition = json_set(condition, '$.kind', CASE json_extract(condition, '$.kind')
    WHEN 'capacity' THEN 'Capacity'
    WHEN 'runtime_failed' THEN 'RuntimeFailed'
    WHEN 'host_offline' THEN 'HostOffline'
END)
WHERE json_extract(condition, '$.kind') IN ('capacity', 'runtime_failed', 'host_offline');

UPDATE workspace_column SET kind = json_set(kind, '$.kind', CASE json_extract(kind, '$.kind')
    WHEN 'shell' THEN 'Shell'
    WHEN 'browser' THEN 'Browser'
END)
WHERE json_extract(kind, '$.kind') IN ('shell', 'browser');

UPDATE project SET opening = (
    SELECT json_group_array(json_set(json(value), '$.kind.kind', CASE json_extract(value, '$.kind.kind')
        WHEN 'shell' THEN 'Shell'
        WHEN 'browser' THEN 'Browser'
        ELSE json_extract(value, '$.kind.kind')
    END))
    FROM json_each(project.opening)
);

UPDATE project SET idle = json_set(idle, '$.kind', CASE json_extract(idle, '$.kind')
    WHEN 'default' THEN 'Default'
    WHEN 'never' THEN 'Never'
    WHEN 'after' THEN 'After'
END)
WHERE json_extract(idle, '$.kind') IN ('default', 'never', 'after');

UPDATE preferences SET document = json_set(document, '$.look', CASE json_extract(document, '$.look')
    WHEN 'auto' THEN 'Auto'
    WHEN 'dark' THEN 'Dark'
    WHEN 'light' THEN 'Light'
END)
WHERE json_extract(document, '$.look') IN ('auto', 'dark', 'light');

UPDATE preferences SET document = json_set(document, '$.keyboard.option_as_meta', CASE json_extract(document, '$.keyboard.option_as_meta')
    WHEN 'left' THEN 'Left'
    WHEN 'both' THEN 'Both'
    WHEN 'off' THEN 'Off'
END)
WHERE json_extract(document, '$.keyboard.option_as_meta') IN ('left', 'both', 'off');
