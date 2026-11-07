-- Accounts, membership and invites. The change log itself lives in each space's Durable Object.

CREATE TABLE users (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE devices (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE members (
  space_id  TEXT NOT NULL,
  user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role      TEXT NOT NULL CHECK (role IN ('Owner', 'Editor', 'Viewer')),
  joined_at INTEGER NOT NULL,
  PRIMARY KEY (space_id, user_id)
);

-- A space has exactly one owner; the first claim wins.
CREATE UNIQUE INDEX members_one_owner ON members(space_id) WHERE role = 'Owner';

-- Only a hash of the token is stored, so a leaked database cannot be replayed as invite links.
CREATE TABLE invites (
  token_hash  TEXT PRIMARY KEY,
  space_id    TEXT NOT NULL,
  role        TEXT NOT NULL CHECK (role IN ('Editor', 'Viewer')),
  created_by  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL,
  -- accepted_at marks the invite as used; accepted_by may later be nulled if that user is deleted.
  accepted_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  accepted_at INTEGER
);

CREATE INDEX invites_space ON invites(space_id);
