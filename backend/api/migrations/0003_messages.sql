-- Text messages in a channel (issue #5). History is channel-local and paged
-- by (created_at, id). Hard delete; an edit stamps `edited_at`. No threads,
-- reactions, or search index in v1. Voice channels have no rows here — the
-- API rejects them as `404`.

CREATE TABLE messages (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id UUID        NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    author_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    content    TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    edited_at  TIMESTAMPTZ
);

-- Newest page and `before`/`after` cursors all walk this order.
CREATE INDEX messages_channel_created_id_idx
    ON messages (channel_id, created_at DESC, id DESC);
