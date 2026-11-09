-- Development schema snapshot. Before 1.0, edit this file in place rather
-- than adding migration scripts. Keep user_version aligned with SCHEMA_VERSION.
-- The Rust initializer supplies the transaction; this file is for empty DBs only.
CREATE TABLE models (
    name TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL,
    backend TEXT NOT NULL,
    ownership TEXT NOT NULL CHECK(ownership IN ('reference','managed')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    request TEXT NOT NULL CHECK(json_valid(request)),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','cancelled','interrupted')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    result TEXT CHECK(result IS NULL OR json_valid(result)),
    error TEXT CHECK(error IS NULL OR json_valid(error))
);
CREATE INDEX jobs_queue ON jobs(status,id);

CREATE TABLE model_catalog (
    id TEXT PRIMARY KEY NOT NULL,
    origin TEXT NOT NULL CHECK(origin IN ('builtin','user')),
    name TEXT NOT NULL,
    family TEXT NOT NULL,
    model_type TEXT NOT NULL CHECK(model_type IN ('text','image','video','audio','music','other')),
    description TEXT NOT NULL,
    repository TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active','deprecated','retired')),
    recommended INTEGER NOT NULL CHECK(recommended IN (0,1)),
    definition TEXT NOT NULL CHECK(json_valid(definition)),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX model_catalog_family ON model_catalog(family);

CREATE TABLE catalog_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    builtin_revision INTEGER NOT NULL
);

CREATE TABLE assets (
    id TEXT PRIMARY KEY NOT NULL,
    job_id INTEGER NOT NULL REFERENCES jobs(id),
    path TEXT UNIQUE NOT NULL,
    sha256 TEXT NOT NULL,
    media_type TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- This is a schema marker, not the application release version. Retain 3 to
-- preserve existing compatible development databases when consolidating SQL.
PRAGMA user_version = 3;
