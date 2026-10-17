-- Link previews (#254). One row per fetch of a URL, never changed: the
-- messages that got it keep it, and a refetch after a day (`previews`
-- module) is a new row for the messages after. The image is a copy in the
-- object store under `previews/`, served by the API.
-- `message_previews` orders up to three of them under a message.
-- `messages.suppress_embeds`: the author removed the previews; edits keep
-- them off.

CREATE TABLE link_previews (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- SHA-256 of `url`: URLs can outgrow a btree key.
    url_hash      BYTEA       NOT NULL,
    url           TEXT        NOT NULL,
    status        TEXT        NOT NULL CHECK (status IN ('ok', 'failed')),
    title         TEXT,
    description   TEXT,
    site_name     TEXT,
    youtube_id    TEXT,
    image_key     TEXT,
    image_width   INTEGER,
    image_height  INTEGER,
    fetched_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (image_key IS NULL OR (image_width > 0 AND image_height > 0))
);

CREATE INDEX link_previews_url_idx ON link_previews (url_hash, fetched_at DESC);
CREATE INDEX link_previews_fetched_idx ON link_previews (fetched_at);

CREATE TABLE message_previews (
    message_id  UUID     NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    preview_id  UUID     NOT NULL REFERENCES link_previews (id) ON DELETE CASCADE,
    position    SMALLINT NOT NULL CHECK (position BETWEEN 0 AND 2),
    PRIMARY KEY (message_id, position)
);

CREATE INDEX message_previews_preview_idx ON message_previews (preview_id);

ALTER TABLE messages ADD COLUMN suppress_embeds BOOLEAN NOT NULL DEFAULT false;
