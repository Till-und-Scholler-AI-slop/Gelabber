-- Moderation (issue #15): persist bans so a kicked user cannot rejoin via
-- invite. `manage_messages` is bit 64 in member_permissions; the column
-- default stays 60 (talk/share/voice/live, no manage flags).

CREATE TABLE server_bans (
    server_id UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    user_id   UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    banned_by UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    banned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (server_id, user_id)
);

CREATE INDEX server_bans_user_id_idx ON server_bans (user_id);
