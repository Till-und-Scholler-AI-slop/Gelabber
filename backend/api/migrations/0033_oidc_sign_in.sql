-- Sign-in through OpenID Connect (v0.10). An account may have no password:
-- it signs in through a provider until its owner sets one.
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- A provider account (issuer-scoped `sub`) attached to a Gelabber account.
-- `email` is the provider's address at the last sign-in, shown in Sicherheit.
CREATE TABLE user_identities (
    provider     TEXT        NOT NULL,
    subject      TEXT        NOT NULL,
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email        TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject),
    UNIQUE (user_id, provider)
);

-- One sign-in or link attempt between `start` and the provider's callback.
-- Only the SHA-256 of `state` and of the browser binding is stored. The PKCE
-- verifier and nonce are kept as they are: the API must send the verifier
-- itself, and the row lives ten minutes. `invitation` is a beta invitation's
-- code for the registration gate (`operator::admission`).
CREATE TABLE oidc_flows (
    state_hash        BYTEA       PRIMARY KEY,
    provider          TEXT        NOT NULL,
    purpose           TEXT        NOT NULL CHECK (purpose IN ('sign_in', 'link')),
    link_user_id      UUID        REFERENCES users (id) ON DELETE CASCADE,
    nonce             TEXT        NOT NULL,
    pkce_verifier     TEXT        NOT NULL,
    binding_hash      BYTEA,
    desktop_port      INTEGER     CHECK (desktop_port BETWEEN 1024 AND 65535),
    desktop_state     TEXT,
    desktop_challenge TEXT,
    redirect          TEXT,
    invitation        TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL,
    CHECK ((binding_hash IS NULL) <> (desktop_challenge IS NULL)),
    CHECK ((desktop_challenge IS NULL) = (desktop_port IS NULL)),
    CHECK (purpose = 'sign_in' OR link_user_id IS NOT NULL)
);
CREATE INDEX oidc_flows_expires_at_idx ON oidc_flows (expires_at);

-- A verified provider answer waiting for its last step: the name of a new
-- account (web and desktop), or the desktop app's exchange. Single use.
CREATE TABLE oidc_results (
    code_hash         BYTEA       PRIMARY KEY,
    provider          TEXT        NOT NULL,
    subject           TEXT        NOT NULL,
    email             TEXT        NOT NULL,
    email_verified    BOOLEAN     NOT NULL,
    name_hint         TEXT,
    purpose           TEXT        NOT NULL CHECK (purpose IN ('sign_in', 'link')),
    link_user_id      UUID        REFERENCES users (id) ON DELETE CASCADE,
    binding_hash      BYTEA,
    desktop_challenge TEXT,
    redirect          TEXT,
    invitation        TEXT,
    signup            BOOLEAN     NOT NULL DEFAULT false,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL,
    CHECK ((binding_hash IS NULL) <> (desktop_challenge IS NULL))
);
CREATE INDEX oidc_results_expires_at_idx ON oidc_results (expires_at);
