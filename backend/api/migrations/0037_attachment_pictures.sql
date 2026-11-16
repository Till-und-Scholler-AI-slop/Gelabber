-- Picture metadata and thumbnails (#574). Width and height are the size as
-- shown (EXIF orientation applied), set when the picture joins a message;
-- the media worker (`attachments::media`) fills in the rest after upload
-- and for older attachments (backfill: every bound picture starts at
-- media_version 0).
ALTER TABLE attachments
    ADD COLUMN width INTEGER CHECK (width > 0),
    ADD COLUMN height INTEGER CHECK (height > 0),
    -- Version 1: the average colour as `#rrggbb`. A later version (e.g. a
    -- ThumbHash) replaces it without another migration.
    ADD COLUMN placeholder TEXT,
    ADD COLUMN placeholder_version SMALLINT,
    ADD COLUMN animated BOOLEAN NOT NULL DEFAULT false,
    -- The size after privacy metadata was removed before binding; becomes
    -- size_bytes when the file joins its message.
    ADD COLUMN stripped_size BIGINT CHECK (stripped_size > 0),
    -- Type of the `<object_key>/t400` and `/t800` thumbnails, NULL if none.
    ADD COLUMN thumb_type TEXT,
    ADD COLUMN media_version SMALLINT NOT NULL DEFAULT 0,
    ADD COLUMN media_attempts SMALLINT NOT NULL DEFAULT 0,
    ADD COLUMN media_retry_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Newest first: a picture just sent goes before the backfill.
CREATE INDEX attachments_media_due_idx ON attachments (created_at DESC)
    WHERE media_version < 1 AND message_id IS NOT NULL
      AND content_type IN ('image/jpeg', 'image/png', 'image/gif', 'image/webp');

-- As 0008, and a picture's thumbnails go with it. They are queued whether
-- or not the worker made them yet: deleting a missing object is harmless,
-- and the worker queues any it writes after the row is gone.
CREATE OR REPLACE FUNCTION attachment_delete_intent() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO storage_cleanup (object_key, retain_until, quota_uploader, quota_day, reserved_bytes)
        VALUES (OLD.object_key, OLD.expires_at + interval '15 minutes', OLD.uploader_id, OLD.quota_day,
            CASE WHEN OLD.quota_state='reserved' THEN OLD.size_bytes ELSE 0 END)
        ON CONFLICT (object_key) DO UPDATE SET
            next_attempt=now(), retain_until=GREATEST(storage_cleanup.retain_until, EXCLUDED.retain_until),
            quota_uploader=EXCLUDED.quota_uploader, quota_day=EXCLUDED.quota_day,
            reserved_bytes=GREATEST(storage_cleanup.reserved_bytes, EXCLUDED.reserved_bytes);
    IF OLD.content_type IN ('image/jpeg', 'image/png', 'image/gif', 'image/webp') THEN
        INSERT INTO storage_cleanup (object_key, retain_until)
            VALUES (OLD.object_key || '/t400', now()), (OLD.object_key || '/t800', now())
            ON CONFLICT (object_key) DO UPDATE SET next_attempt=now();
    END IF;
    RETURN OLD;
END $$;
