-- The server overview's "Geteilt" (GEL-69) takes each channel's newest
-- sent files; without this it would sort every file of the server.
CREATE INDEX attachments_sent_by_channel_idx
    ON attachments (channel_id, created_at DESC, id DESC)
    WHERE message_id IS NOT NULL;
