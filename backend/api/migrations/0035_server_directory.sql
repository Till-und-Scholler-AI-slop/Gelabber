-- Directory listings (opt-in per server) and join requests (v0.10 discovery).
-- A row exists once the owner saved the listing form; `listed` is the owner's
-- switch. An operator's delisting is `servers.delisted_at` (0032). Visible =
-- listed AND the server is neither delisted nor blocked.

CREATE TABLE server_listings (
    server_id    UUID        PRIMARY KEY REFERENCES servers (id) ON DELETE CASCADE,
    listed       BOOLEAN     NOT NULL DEFAULT false,
    description  TEXT        NOT NULL DEFAULT ''
                 CHECK (char_length(description) <= 300),
    language     TEXT        NOT NULL CHECK (language ~ '^[a-z]{2}$'),
    join_mode    TEXT        NOT NULL DEFAULT 'request'
                 CHECK (join_mode IN ('open', 'request')),
    -- First time `listed` became true; the directory's tie-break order.
    listed_at    TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT server_listings_listed_has_description
        CHECK (NOT listed OR char_length(description) >= 1)
);

CREATE INDEX server_listings_listed_idx
    ON server_listings (language, listed_at)
    WHERE listed;

-- One row per (server, person). Accepted and withdrawn requests are deleted;
-- a declined one stays as the 7-day cooldown and is replaced by a new request
-- after that.
CREATE TABLE server_join_requests (
    server_id   UUID        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    message     TEXT        NOT NULL DEFAULT '' CHECK (char_length(message) <= 300),
    status      TEXT        NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'declined')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at  TIMESTAMPTZ,
    decided_by  UUID        REFERENCES users (id) ON DELETE SET NULL,
    PRIMARY KEY (server_id, user_id)
);

CREATE INDEX server_join_requests_pending_idx
    ON server_join_requests (server_id, created_at) WHERE status = 'pending';
CREATE INDEX server_join_requests_user_idx
    ON server_join_requests (user_id) WHERE status = 'pending';
