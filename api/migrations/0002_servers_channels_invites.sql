-- Servers → categories → channels, membership and invite links (issue #4).
-- Roles are coarse (owner + member): the owner is `servers.owner_id`, every
-- other row in `server_members` is a member. Permissions are one bitmask per
-- server that applies to all members; the owner implicitly has everything.
-- No per-channel overwrites in v1.

CREATE TABLE servers (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name               TEXT        NOT NULL,
    owner_id           UUID        NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    -- Bitmask of the flags members hold (see api/src/servers/permissions.rs):
    -- 1 manage_server, 2 manage_channels, 4 send_messages, 8 send_files,
    -- 16 join_voice, 32 go_live.
    member_permissions INTEGER     NOT NULL DEFAULT 60,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX servers_owner_id_idx ON servers (owner_id);

CREATE TABLE server_members (
    server_id UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    user_id   UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (server_id, user_id)
);

-- The sidebar lists a user's servers in join order.
CREATE INDEX server_members_user_id_joined_at_idx ON server_members (user_id, joined_at);

CREATE TABLE categories (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id  UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    name       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX categories_server_id_idx ON categories (server_id, created_at);

CREATE TABLE channels (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id   UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    -- Deleting a category leaves its channels in place, uncategorised.
    category_id UUID        REFERENCES categories (id) ON DELETE SET NULL,
    name        TEXT        NOT NULL,
    kind        TEXT        NOT NULL CHECK (kind IN ('text', 'voice')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX channels_server_id_idx ON channels (server_id, created_at);
CREATE INDEX channels_category_id_idx ON channels (category_id);

-- The code is the URL path segment of the invite link. `max_uses`/`expires_at`
-- NULL = unlimited/never; `uses` is bumped inside the join transaction.
CREATE TABLE invites (
    code       TEXT        PRIMARY KEY,
    server_id  UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    created_by UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    max_uses   INTEGER     CHECK (max_uses IS NULL OR max_uses > 0),
    uses       INTEGER     NOT NULL DEFAULT 0
);

CREATE INDEX invites_server_id_idx ON invites (server_id, created_at);
