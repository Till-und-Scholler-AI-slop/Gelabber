-- Issue #616: "push me when someone starts a call", an opt-in per server.
-- A voice channel has no notification setting of its own, so the switch
-- lives on the server's row; a channel's or DM's row never carries it.
ALTER TABLE notification_settings
    ADD COLUMN voice BOOLEAN NOT NULL DEFAULT false,
    ADD CONSTRAINT notification_settings_voice_check CHECK (NOT voice OR scope_kind = 'server');
