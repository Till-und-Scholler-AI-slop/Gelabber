-- Operator area (v0.10): disabled accounts, blocked/delisted servers, beta
-- invitations, reports, and the operator audit trail.

ALTER TABLE users
    ADD COLUMN disabled_at     TIMESTAMPTZ,
    ADD COLUMN disabled_reason TEXT CHECK (char_length(disabled_reason) <= 500);

ALTER TABLE servers
    ADD COLUMN blocked_at     TIMESTAMPTZ,
    ADD COLUMN blocked_reason TEXT CHECK (char_length(blocked_reason) <= 500),
    -- Set by an operator. The directory (discovery package) never lists a
    -- server while this is set, and its owner cannot list it again.
    ADD COLUMN delisted_at    TIMESTAMPTZ;

CREATE TABLE beta_invitations (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Same alphabet and length as server invites (servers::invite::generate_code).
    code       TEXT        NOT NULL UNIQUE,
    -- Lower-cased. NULL: anyone holding the link.
    email      TEXT,
    note       TEXT        CHECK (char_length(note) <= 200),
    created_by UUID        REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    used_by    UUID        REFERENCES users (id) ON DELETE SET NULL,
    used_at    TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    CHECK (used_by IS NULL OR used_at IS NOT NULL)
);
CREATE INDEX beta_invitations_created_idx ON beta_invitations (created_at DESC);

-- Shared with the discovery package, whose report button writes here
-- (operator::reports::file). `snapshot` keeps what the reporter saw.
CREATE TABLE reports (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id       UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    reporter_id     UUID        REFERENCES users (id) ON DELETE SET NULL,
    reason          TEXT        NOT NULL
                                CHECK (reason IN ('spam', 'harassment', 'sexual', 'illegal', 'other')),
    details         TEXT        NOT NULL DEFAULT '' CHECK (char_length(details) <= 1000),
    snapshot        JSONB,
    status          TEXT        NOT NULL DEFAULT 'open'
                                CHECK (status IN ('open', 'dismissed', 'actioned')),
    resolved_by     UUID        REFERENCES users (id) ON DELETE SET NULL,
    resolved_at     TIMESTAMPTZ,
    resolution_note TEXT        CHECK (char_length(resolution_note) <= 500),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((status = 'open') = (resolved_at IS NULL))
);
CREATE INDEX reports_open_idx   ON reports (created_at) WHERE status = 'open';
CREATE INDEX reports_server_idx ON reports (server_id, status);
-- One open report per person and server; a repeat returns the open one.
CREATE UNIQUE INDEX reports_one_open_per_reporter
    ON reports (server_id, reporter_id) WHERE status = 'open';

CREATE TABLE operator_audit (
    id                BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operator_id       UUID        REFERENCES users (id) ON DELETE SET NULL,
    operator_email    TEXT        NOT NULL,
    action            TEXT        NOT NULL,
    -- No foreign keys: a row outlives the account, server or report it names.
    target_user       UUID,
    target_server     UUID,
    target_report     UUID,
    target_invitation UUID,
    detail            JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX operator_audit_created_idx ON operator_audit (created_at DESC);

-- Auslastung counts messages by time across all channels.
CREATE INDEX messages_created_at_idx ON messages (created_at);
