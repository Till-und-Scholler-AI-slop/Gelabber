-- Account mail (#195): confirmed addresses and single-use mailed links.

-- NULL: the address was never confirmed. While account mail is on, such an
-- account cannot sign in until its owner opens a confirmation link (or sets
-- a password through a reset link, which proves the same).
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;

-- Accounts from before account mail keep working when an operator turns it
-- on later: they count as confirmed since they signed up.
UPDATE users SET email_verified_at = created_at;

-- Like `sessions`: the mail carries the random token, only its SHA-256 is
-- stored. A row is deleted when its link is used, so each works once.
CREATE TABLE account_tokens (
    token_hash BYTEA       PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose    TEXT        NOT NULL CHECK (purpose IN ('verify_email', 'reset_password')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX account_tokens_user_purpose_idx ON account_tokens (user_id, purpose);
CREATE INDEX account_tokens_expires_at_idx ON account_tokens (expires_at);
