-- Hosts mount their own guest tools now and refuse no workspace for an
-- older iglu: what they refused is tried again.
UPDATE workspace SET condition = NULL
WHERE json_extract(condition, '$.code') = 'image_incompatible';
