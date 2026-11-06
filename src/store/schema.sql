-- The cache schema. A change here needs a new user_version, and the same
-- value in SCHEMA_VERSION in mod.rs. On a mismatch the store drops every
-- table except ui_state and runs this file again.
PRAGMA user_version = 2;

CREATE TABLE IF NOT EXISTS repos(
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  squash_allowed INT,
  merge_allowed INT,
  rebase_allowed INT,
  default_branch TEXT,
  PRIMARY KEY(owner, name)
);

CREATE TABLE IF NOT EXISTS pull_requests(
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  number INT NOT NULL,
  scope TEXT,
  state TEXT,
  updated_at TEXT,
  synced_at TEXT,
  json BLOB,
  PRIMARY KEY(owner, name, number)
);

CREATE INDEX IF NOT EXISTS pr_open ON pull_requests(state, scope);

CREATE TABLE IF NOT EXISTS pr_details(
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  number INT NOT NULL,
  fetched_at TEXT,
  head_oid TEXT,
  json BLOB,
  PRIMARY KEY(owner, name, number)
);

CREATE TABLE IF NOT EXISTS pr_files(
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  number INT NOT NULL,
  fetched_at TEXT,
  head_oid TEXT,
  json BLOB,
  PRIMARY KEY(owner, name, number)
);

CREATE TABLE IF NOT EXISTS local_state(
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  scanned_at TEXT,
  json BLOB,
  PRIMARY KEY(owner, name)
);

CREATE TABLE IF NOT EXISTS sync_state(
  scope TEXT NOT NULL,
  repo_set TEXT NOT NULL,
  last_full TEXT,
  last_delta TEXT,
  last_error TEXT,
  PRIMARY KEY(scope, repo_set)
);

CREATE TABLE IF NOT EXISTS ui_state(
  key TEXT PRIMARY KEY,
  value TEXT
);
