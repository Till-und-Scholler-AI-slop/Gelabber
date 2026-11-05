-- Durable quota and deletion intent survive message/channel/server cascades.
CREATE TABLE upload_daily_usage (
    uploader_id UUID NOT NULL,
    day DATE NOT NULL,
    reserved BIGINT NOT NULL DEFAULT 0 CHECK (reserved >= 0),
    consumed BIGINT NOT NULL DEFAULT 0 CHECK (consumed >= 0),
    PRIMARY KEY (uploader_id, day)
);
ALTER TABLE attachments
    ADD COLUMN expiry_retry_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '10 minutes'),
    ADD COLUMN quota_day DATE NOT NULL DEFAULT ((now() AT TIME ZONE 'UTC')::date),
    ADD COLUMN quota_state TEXT NOT NULL DEFAULT 'reserved'
        CHECK (quota_state IN ('reserved', 'consumed', 'unused'));
UPDATE attachments SET expires_at=created_at + interval '10 minutes',
    quota_day=(created_at AT TIME ZONE 'UTC')::date,
    quota_state=CASE WHEN message_id IS NULL THEN 'reserved' ELSE 'consumed' END;
INSERT INTO upload_daily_usage (uploader_id, day, reserved, consumed)
    SELECT uploader_id, quota_day,
           COALESCE(SUM(size_bytes) FILTER (WHERE quota_state='reserved'),0),
           COALESCE(SUM(size_bytes) FILTER (WHERE quota_state='consumed'),0)
    FROM attachments GROUP BY uploader_id, quota_day;
CREATE INDEX attachments_expiry_idx ON attachments (expires_at, id)
    WHERE message_id IS NULL;

CREATE TABLE storage_cleanup (
    object_key TEXT PRIMARY KEY,
    next_attempt TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Repeat deletion until the issued PUT URL and an in-flight grace expire.
    retain_until TIMESTAMPTZ NOT NULL,
    claim UUID,
    claimed_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    quota_uploader UUID,
    quota_day DATE,
    reserved_bytes BIGINT NOT NULL DEFAULT 0 CHECK (reserved_bytes >= 0)
);
CREATE INDEX storage_cleanup_due_idx ON storage_cleanup (next_attempt);

-- Also account for old binary inserts during a short rollback. They cannot
-- enforce the new cap, but must not corrupt the ledger or block cascades.
CREATE FUNCTION attachment_quota_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.message_id IS NOT NULL THEN NEW.quota_state='consumed'; END IF;
    INSERT INTO upload_daily_usage (uploader_id, day, reserved, consumed)
        VALUES (NEW.uploader_id, NEW.quota_day,
            CASE WHEN NEW.quota_state='reserved' THEN NEW.size_bytes ELSE 0 END,
            CASE WHEN NEW.quota_state='consumed' THEN NEW.size_bytes ELSE 0 END)
        ON CONFLICT (uploader_id, day) DO UPDATE SET
            reserved=upload_daily_usage.reserved+EXCLUDED.reserved,
            consumed=upload_daily_usage.consumed+EXCLUDED.consumed;
    RETURN NEW;
END $$;
CREATE TRIGGER attachment_quota_create BEFORE INSERT ON attachments
    FOR EACH ROW EXECUTE FUNCTION attachment_quota_insert();

CREATE FUNCTION attachment_quota_transition() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.message_id IS NOT NULL AND OLD.quota_state='reserved' THEN
        NEW.quota_state='consumed';
    END IF;
    IF OLD.quota_state='reserved' AND NEW.quota_state<>'reserved' THEN
        UPDATE upload_daily_usage SET reserved=reserved-OLD.size_bytes,
            consumed=consumed+CASE WHEN NEW.quota_state='consumed' THEN OLD.size_bytes ELSE 0 END
            WHERE uploader_id=OLD.uploader_id AND day=OLD.quota_day;
    ELSIF OLD.quota_state<>NEW.quota_state THEN
        RAISE EXCEPTION 'invalid attachment quota transition';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER attachment_quota_update BEFORE UPDATE OF quota_state, message_id ON attachments
    FOR EACH ROW EXECUTE FUNCTION attachment_quota_transition();

CREATE FUNCTION attachment_delete_intent() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- A cascade cannot HEAD the object. Conservatively charge an unknown PUT;
    -- expiry cleanup refunds only after a confirmed missing object.
    -- Do not take uploader ledger locks while a cascade holds multiple
    -- parent/attachment rows: two cascades can visit uploaders in reverse order.
    -- Transfer pending reservations in the worker, one ledger per transaction.
    INSERT INTO storage_cleanup (object_key, retain_until, quota_uploader, quota_day, reserved_bytes)
        VALUES (OLD.object_key, OLD.expires_at + interval '15 minutes', OLD.uploader_id, OLD.quota_day,
            CASE WHEN OLD.quota_state='reserved' THEN OLD.size_bytes ELSE 0 END)
        ON CONFLICT (object_key) DO UPDATE SET
            next_attempt=now(), retain_until=GREATEST(storage_cleanup.retain_until, EXCLUDED.retain_until),
            quota_uploader=EXCLUDED.quota_uploader, quota_day=EXCLUDED.quota_day,
            reserved_bytes=GREATEST(storage_cleanup.reserved_bytes, EXCLUDED.reserved_bytes);
    RETURN OLD;
END $$;
CREATE TRIGGER attachment_cleanup BEFORE DELETE ON attachments
    FOR EACH ROW EXECUTE FUNCTION attachment_delete_intent();
