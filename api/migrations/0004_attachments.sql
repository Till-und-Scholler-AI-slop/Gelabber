-- File attachments (issue #7). Bytes live in MinIO; this table is metadata
-- only. A row is created at presign (`message_id` null) and bound when the
-- chat message is posted. Hard-delete of the message drops the row.
-- No public object URLs — the API is the auth gate.

CREATE TABLE attachments (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id    UUID        REFERENCES messages (id) ON DELETE CASCADE,
    channel_id    UUID        NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    uploader_id   UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    object_key    TEXT        NOT NULL UNIQUE,
    filename      TEXT        NOT NULL,
    content_type  TEXT        NOT NULL,
    size_bytes    BIGINT      NOT NULL CHECK (size_bytes > 0),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX attachments_message_id_idx
    ON attachments (message_id);

CREATE INDEX attachments_pending_uploader_idx
    ON attachments (channel_id, uploader_id)
    WHERE message_id IS NULL;
