-- iglud's schema. SQLite owns intent; Incus owns what exists.

CREATE TABLE principal (
    id                  TEXT PRIMARY KEY,
    issuer              TEXT NOT NULL,
    subject             TEXT NOT NULL,
    email               TEXT,
    name                TEXT,
    status              TEXT NOT NULL CHECK (status IN ('active', 'disabled')),
    secrets_generation  INTEGER NOT NULL DEFAULT 1,
    created_at          INTEGER NOT NULL,
    UNIQUE (issuer, subject)
) STRICT;

CREATE TABLE web_session (
    token_hash    TEXT PRIMARY KEY,
    principal_id  TEXT NOT NULL REFERENCES principal (id),
    kind          TEXT NOT NULL CHECK (kind IN ('console', 'preview', 'api')),
    csrf          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE login (
    state          TEXT PRIMARY KEY,
    kind           TEXT NOT NULL CHECK (kind IN ('console', 'preview')),
    nonce          TEXT NOT NULL,
    pkce_verifier  TEXT NOT NULL,
    return_to      TEXT NOT NULL,
    created_at     INTEGER NOT NULL
) STRICT;

CREATE TABLE cli_code (
    code_hash     TEXT PRIMARY KEY,
    principal_id  TEXT NOT NULL REFERENCES principal (id),
    challenge     TEXT NOT NULL,
    expires_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE environment (
    id          TEXT PRIMARY KEY,
    owner_id    TEXT NOT NULL REFERENCES principal (id),
    name        TEXT NOT NULL,
    source      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE (owner_id, name)
) STRICT;

CREATE TABLE env_revision (
    id              TEXT PRIMARY KEY,
    environment_id  TEXT NOT NULL REFERENCES environment (id),
    status          TEXT NOT NULL CHECK (status IN ('building', 'ready', 'failed')),
    built           TEXT,
    log_tail        TEXT,
    created_at      INTEGER NOT NULL,
    finished_at     INTEGER
) STRICT;

CREATE TABLE workspace (
    id               TEXT PRIMARY KEY,
    owner_id         TEXT NOT NULL REFERENCES principal (id),
    host_id          TEXT NOT NULL,
    env_revision_id  TEXT NOT NULL REFERENCES env_revision (id),
    name             TEXT NOT NULL,
    repo             TEXT NOT NULL,
    branch           TEXT NOT NULL,
    base             TEXT,
    desired          TEXT NOT NULL,
    revision         INTEGER NOT NULL,
    observed         TEXT,
    observed_at      INTEGER,
    memory           INTEGER,
    condition        TEXT,
    create_key       TEXT,
    create_hash      TEXT,
    created_at       INTEGER NOT NULL,
    deleted_at       INTEGER,
    UNIQUE (owner_id, create_key)
) STRICT;

CREATE UNIQUE INDEX workspace_live_name ON workspace (owner_id, name) WHERE deleted_at IS NULL;

CREATE TABLE attention (
    workspace_id  TEXT NOT NULL REFERENCES workspace (id),
    session       TEXT NOT NULL,
    state         TEXT NOT NULL,
    summary       TEXT NOT NULL,
    updated_at    INTEGER NOT NULL,
    seen_at       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, session)
) STRICT;

-- Names are owned separately from routes so they can later be rebound. A
-- name that ever served a service worker must never move to another owner.
CREATE TABLE route_name (
    name        TEXT PRIMARY KEY,
    owner_id    TEXT NOT NULL REFERENCES principal (id),
    generation  INTEGER NOT NULL,
    sw_seen     INTEGER NOT NULL DEFAULT 0,
    retired_at  INTEGER
) STRICT;

CREATE TABLE route (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspace (id),
    name          TEXT NOT NULL UNIQUE REFERENCES route_name (name),
    port          INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    UNIQUE (workspace_id, port)
) STRICT;

CREATE TABLE secret (
    id          TEXT PRIMARY KEY,
    owner_id    TEXT NOT NULL REFERENCES principal (id),
    name        TEXT NOT NULL,
    target      TEXT NOT NULL,
    nonce       BLOB NOT NULL,
    ciphertext  BLOB NOT NULL,
    updated_at  INTEGER NOT NULL,
    UNIQUE (owner_id, name)
) STRICT;

CREATE TABLE activity (
    id            INTEGER PRIMARY KEY,
    workspace_id  TEXT REFERENCES workspace (id),
    actor_id      TEXT,
    kind          TEXT NOT NULL,
    detail        TEXT NOT NULL,
    at            INTEGER NOT NULL
) STRICT;

CREATE INDEX activity_by_workspace ON activity (workspace_id, at);
