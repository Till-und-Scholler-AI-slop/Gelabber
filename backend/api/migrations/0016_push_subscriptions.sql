-- Web Push (issue #243): a browser's subscription, tied to the session that
-- made it. Signing out, a revoked or an expired session ends the pushes.
CREATE TABLE push_subscriptions (
    endpoint     TEXT        PRIMARY KEY,
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    session_hash BYTEA       NOT NULL REFERENCES sessions (token_hash) ON DELETE CASCADE,
    p256dh       TEXT        NOT NULL,
    auth         TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_ok_at   TIMESTAMPTZ
);

CREATE INDEX push_subscriptions_user_id_idx ON push_subscriptions (user_id);
CREATE INDEX push_subscriptions_session_hash_idx ON push_subscriptions (session_hash);
