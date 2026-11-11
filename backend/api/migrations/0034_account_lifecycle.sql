-- Account lifecycle (v0.10): signed-in devices, account deletion and the
-- data export.

-- Signed-in devices (B2). `id` is what clients see and address; the token
-- hash never leaves the server. Device fields are parsed from User-Agent
-- at sign-in (auth::device), stored as codes so each client words them in
-- the reader's language. `last_used_at` moves with the sliding expiry
-- (session::touch), so it is accurate to one renewal step (at most an hour).
ALTER TABLE sessions
    ADD COLUMN id UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN device_client TEXT NOT NULL DEFAULT 'unknown'
        CHECK (device_client IN ('desktop', 'firefox', 'chrome', 'edge', 'safari', 'browser', 'unknown')),
    ADD COLUMN device_os TEXT NOT NULL DEFAULT 'other'
        CHECK (device_os IN ('linux', 'windows', 'macos', 'android', 'ios', 'other'));
UPDATE sessions SET last_used_at = created_at;
CREATE UNIQUE INDEX sessions_id_unique ON sessions (id);

-- Account deletion (two phases, account::deletion). Set when the owner
-- confirmed; the purge worker removes the row once messages, DMs and files
-- are gone. While set, nobody can sign in to or address the account.
ALTER TABLE users ADD COLUMN deleted_at TIMESTAMPTZ;
CREATE INDEX users_deleted_at_idx ON users (deleted_at) WHERE deleted_at IS NOT NULL;

-- A ban outlives the account of whoever issued it.
ALTER TABLE server_bans ALTER COLUMN banned_by DROP NOT NULL;
ALTER TABLE server_bans DROP CONSTRAINT server_bans_banned_by_fkey;
ALTER TABLE server_bans ADD CONSTRAINT server_bans_banned_by_fkey
    FOREIGN KEY (banned_by) REFERENCES users (id) ON DELETE SET NULL;

-- Data export: at most one per account, replaced by the next request.
CREATE TABLE account_exports (
    user_id       UUID        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    state         TEXT        NOT NULL CHECK (state IN ('pending', 'running', 'ready', 'failed')),
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    claim         UUID,
    claimed_until TIMESTAMPTZ,
    attempts      INTEGER     NOT NULL DEFAULT 0,
    finished_at   TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,
    object_key    TEXT        UNIQUE,
    size_bytes    BIGINT      CHECK (size_bytes >= 0),
    CONSTRAINT account_exports_ready_shape CHECK (
        (state = 'ready') = (object_key IS NOT NULL AND expires_at IS NOT NULL AND size_bytes IS NOT NULL))
);
CREATE INDEX account_exports_due_idx ON account_exports (requested_at)
    WHERE state IN ('pending', 'running');
CREATE INDEX account_exports_expiry_idx ON account_exports (expires_at) WHERE state = 'ready';

-- The archive object goes with its row, the same way attachments do (0008).
CREATE FUNCTION account_export_cleanup() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.object_key IS NOT NULL
       AND (TG_OP = 'DELETE' OR NEW.object_key IS DISTINCT FROM OLD.object_key) THEN
        INSERT INTO storage_cleanup (object_key, retain_until) VALUES (OLD.object_key, now())
            ON CONFLICT (object_key) DO UPDATE SET next_attempt = now();
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END $$;
CREATE TRIGGER account_export_cleanup BEFORE DELETE OR UPDATE OF object_key ON account_exports
    FOR EACH ROW EXECUTE FUNCTION account_export_cleanup();
