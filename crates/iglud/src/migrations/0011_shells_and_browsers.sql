-- Columns are shells and browsers; agents are run from a shell. A column
-- that ran an agent or a command opens a shell, under the same name.
UPDATE workspace_column SET kind = '{"kind":"shell"}'
WHERE json_extract(kind, '$.kind') NOT IN ('shell', 'browser');
ALTER TABLE workspace_column DROP COLUMN prompt;

-- Projects open with their shells and browser alone, or one shell.
UPDATE project SET opening = (
    SELECT json_group_array(json(value)) FROM json_each(project.opening)
    WHERE json_extract(value, '$.kind.kind') IN ('shell', 'browser')
);
UPDATE project SET opening = '[{"kind":{"kind":"shell"},"width":"half"}]'
WHERE json_array_length(opening) = 0;
ALTER TABLE project DROP COLUMN agent;

UPDATE env_revision SET built = json_remove(built, '$.agents') WHERE built IS NOT NULL;
