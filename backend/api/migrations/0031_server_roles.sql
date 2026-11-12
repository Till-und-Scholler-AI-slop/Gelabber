-- v0.10 roles. "Alle Mitglieder" (the base role every member holds) stays
-- servers.member_permissions; custom roles are rows here. The owner is
-- servers.owner_id and holds everything. Bit 256 = moderate_members.

-- Kick and ban move out of manage_server into moderate_members: whoever
-- could kick and ban before keeps that.
UPDATE servers SET member_permissions = member_permissions | 256
 WHERE member_permissions & 1 <> 0;

CREATE TABLE server_roles (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id   UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    name        TEXT        NOT NULL,
    color       TEXT        NOT NULL
                CHECK (color IN ('amber', 'blue', 'green', 'pink', 'violet', 'coral')),
    permissions INTEGER     NOT NULL DEFAULT 0,
    position    INTEGER     NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, server_id)
);
CREATE UNIQUE INDEX server_roles_name_idx ON server_roles (server_id, lower(name));
CREATE INDEX server_roles_position_idx ON server_roles (server_id, position);

-- Leaving, kick, ban and server delete drop the rows with the membership.
CREATE TABLE server_member_roles (
    server_id UUID NOT NULL,
    user_id   UUID NOT NULL,
    role_id   UUID NOT NULL,
    PRIMARY KEY (server_id, user_id, role_id),
    FOREIGN KEY (server_id, user_id) REFERENCES server_members (server_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (role_id, server_id) REFERENCES server_roles (id, server_id) ON DELETE CASCADE
);
CREATE INDEX server_member_roles_role_idx ON server_member_roles (role_id);

-- A limited channel is seen by the owner and by holders of one of its roles.
-- It stays limited when its last role is deleted (then only the owner sees it).
ALTER TABLE channels ADD COLUMN limited BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE channels ADD CONSTRAINT channels_id_server_key UNIQUE (id, server_id);

CREATE TABLE channel_roles (
    server_id  UUID NOT NULL,
    channel_id UUID NOT NULL,
    role_id    UUID NOT NULL,
    PRIMARY KEY (channel_id, role_id),
    FOREIGN KEY (channel_id, server_id) REFERENCES channels (id, server_id) ON DELETE CASCADE,
    FOREIGN KEY (role_id, server_id) REFERENCES server_roles (id, server_id) ON DELETE CASCADE
);
CREATE INDEX channel_roles_role_idx ON channel_roles (role_id);

-- Base + role bits of a member. Owner handling stays in Rust.
CREATE FUNCTION server_member_bits(p_server UUID, p_user UUID) RETURNS INTEGER
LANGUAGE sql STABLE AS $$
  SELECT s.member_permissions | COALESCE(
    (SELECT bit_or(r.permissions) FROM server_member_roles mr
       JOIN server_roles r ON r.id = mr.role_id
      WHERE mr.server_id = p_server AND mr.user_id = p_user), 0)
  FROM servers s WHERE s.id = p_server
$$;

-- Whether p_user may see p_channel, given they are a member of its server.
-- DMs (no server) are left to channel_members.
CREATE FUNCTION channel_visible_to(p_channel UUID, p_user UUID) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
  SELECT c.server_id IS NULL OR NOT c.limited OR s.owner_id = p_user
      OR EXISTS (SELECT 1 FROM channel_roles cr
                   JOIN server_member_roles mr
                     ON mr.role_id = cr.role_id AND mr.server_id = cr.server_id
                    AND mr.user_id = p_user
                  WHERE cr.channel_id = c.id)
  FROM channels c LEFT JOIN servers s ON s.id = c.server_id
  WHERE c.id = p_channel
$$;
