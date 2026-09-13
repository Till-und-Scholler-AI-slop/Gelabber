-- Users and cookie sessions (issue #3). E-mail + Argon2id password only;
-- no OAuth/JWT/magic-link tables in v1.

CREATE TABLE users (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Stored lower-cased; the unique index is what enforces one account per
    -- address regardless of how the user typed it.
    email         TEXT        NOT NULL,
    name          TEXT        NOT NULL,
    avatar_url    TEXT,
    -- PHC string ($argon2id$v=19$m=...,t=...,p=...$salt$hash).
    password_hash TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_email_unique ON users (email);

-- One row per browser session. The cookie carries the random token; only
-- its SHA-256 lands here, so a database dump cannot be replayed as a cookie.
CREATE TABLE sessions (
    token_hash BYTEA       PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);
