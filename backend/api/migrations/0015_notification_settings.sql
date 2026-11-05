-- Per-server and per-channel notification settings (issue #242). A DM is a
-- channel. `level` NULL inherits: a channel takes its server's, a server or
-- DM falls back to 'all'. A mute holds while `muted` and `muted_until` is
-- NULL ("until I turn it back on") or later than now; an expired one is
-- simply ignored, so nothing has to clean it up for it to end on time.
CREATE TABLE notification_settings (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('server', 'channel')),
    scope_id UUID NOT NULL,
    level TEXT CHECK (level IN ('all', 'mentions', 'none')),
    muted BOOLEAN NOT NULL DEFAULT false,
    muted_until TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, scope_kind, scope_id)
);

-- A deleted server or channel takes its settings with it.
CREATE FUNCTION notification_settings_forget_server() RETURNS trigger AS $$
BEGIN
    DELETE FROM notification_settings WHERE scope_kind = 'server' AND scope_id = OLD.id;
    RETURN OLD;
END $$ LANGUAGE plpgsql;
CREATE TRIGGER servers_forget_notification_settings
    AFTER DELETE ON servers FOR EACH ROW EXECUTE FUNCTION notification_settings_forget_server();

CREATE FUNCTION notification_settings_forget_channel() RETURNS trigger AS $$
BEGIN
    DELETE FROM notification_settings WHERE scope_kind = 'channel' AND scope_id = OLD.id;
    RETURN OLD;
END $$ LANGUAGE plpgsql;
CREATE TRIGGER channels_forget_notification_settings
    AFTER DELETE ON channels FOR EACH ROW EXECUTE FUNCTION notification_settings_forget_channel();
