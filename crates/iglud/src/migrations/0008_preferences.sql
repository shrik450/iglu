-- What each person chose for how the console looks and takes keys: one JSON
-- document, which fills in defaults for what it doesn't have, so a setting
-- added later needs no migration.
CREATE TABLE preferences (
    principal_id  TEXT PRIMARY KEY REFERENCES principal (id),
    document      TEXT NOT NULL
);
