-- Feedback: entries from signed-in people and from visitors of the public
-- form, their threads with the operators, and the audit target.

CREATE TABLE feedback (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    kind                TEXT        NOT NULL CHECK (kind IN ('idea', 'problem', 'praise')),
    -- The sender's account. NULL for a visitor, and once the account is
    -- deleted: the entry stays for the operators, as reports do.
    user_id             UUID        REFERENCES users (id) ON DELETE SET NULL,
    -- A visitor's address (stored as validate::new_email returns it) and the
    -- language of the form they used. NULL for entries from an account.
    visitor_email       TEXT        CHECK (char_length(visitor_email) <= 254),
    visitor_locale      TEXT        CHECK (visitor_locale IN ('de', 'en')),
    -- What the client sent along, and nothing else.
    client              TEXT        NOT NULL CHECK (client IN ('web', 'desktop', 'form')),
    os                  TEXT        CHECK (os IN ('linux', 'windows', 'macos', 'android', 'ios', 'other')),
    app_version         TEXT        CHECK (char_length(app_version) <= 32),
    locale              TEXT        NOT NULL CHECK (locale IN ('de', 'en')),
    page                TEXT        CHECK (char_length(page) <= 300),
    body                TEXT        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 4000),
    image_key           TEXT        UNIQUE,
    image_type          TEXT        CHECK (image_type IN ('image/png', 'image/jpeg', 'image/webp')),
    image_bytes         INTEGER     CHECK (image_bytes BETWEEN 1 AND 5242880),
    status              TEXT        NOT NULL DEFAULT 'open'
                                    CHECK (status IN ('pending', 'open', 'done')),
    -- Public form only: SHA-256 of the confirmation token, kept until it
    -- runs out so that opening the link again lands on the thank-you page.
    confirm_hash        BYTEA       UNIQUE,
    confirm_expires_at  TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Moves with every message, confirmation and reopening; the lists sort by it.
    last_activity_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    done_at             TIMESTAMPTZ,
    done_by             UUID        REFERENCES users (id) ON DELETE SET NULL,
    -- Set by the first operator reply the person has not seen, cleared when
    -- they open the entry. NULL: nothing unread.
    person_unread_since TIMESTAMPTZ,
    CHECK ((client = 'form') = (visitor_email IS NOT NULL)),
    CHECK ((client = 'form') = (visitor_locale IS NOT NULL)),
    CHECK (client = 'form' OR (os IS NOT NULL AND app_version IS NOT NULL AND page IS NOT NULL)),
    CHECK (client <> 'form' OR image_key IS NULL),
    CHECK ((image_key IS NULL) = (image_type IS NULL)
           AND (image_key IS NULL) = (image_bytes IS NULL)),
    CHECK ((confirm_hash IS NULL) = (confirm_expires_at IS NULL)),
    CHECK (status <> 'pending' OR (client = 'form' AND confirm_hash IS NOT NULL)),
    CHECK ((status = 'done') = (done_at IS NOT NULL))
);
-- Betrieb: open or done, newest activity first; pending entries never show.
CREATE INDEX feedback_list_idx ON feedback (status, last_activity_at DESC, id DESC)
    WHERE status <> 'pending';
-- Einstellungen → Feedback, and the account delete's SET NULL.
CREATE INDEX feedback_user_idx ON feedback (user_id, last_activity_at DESC)
    WHERE user_id IS NOT NULL;
-- The unread count of the status route.
CREATE INDEX feedback_unread_idx ON feedback (user_id)
    WHERE person_unread_since IS NOT NULL;
-- The sweeper: pending entries and confirmation hashes that ran out.
CREATE INDEX feedback_confirm_idx ON feedback (confirm_expires_at)
    WHERE confirm_expires_at IS NOT NULL;
CREATE INDEX feedback_done_by_idx ON feedback (done_by) WHERE done_by IS NOT NULL;

CREATE TABLE feedback_messages (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    feedback_id UUID        NOT NULL REFERENCES feedback (id) ON DELETE CASCADE,
    -- Who wrote it, kept apart from author_id: a reply stays an operator's
    -- reply after that operator's account is gone.
    author_kind TEXT        NOT NULL CHECK (author_kind IN ('person', 'operator')),
    author_id   UUID        REFERENCES users (id) ON DELETE SET NULL,
    body        TEXT        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 4000),
    -- Operator replies to a visitor go out by mail; NULL for every other message.
    mail_state  TEXT        CHECK (mail_state IN ('pending', 'sent', 'failed')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (mail_state IS NULL OR author_kind = 'operator')
);
CREATE INDEX feedback_messages_thread_idx ON feedback_messages (feedback_id, created_at, id);
CREATE INDEX feedback_messages_author_idx ON feedback_messages (author_id)
    WHERE author_id IS NOT NULL;

-- The image goes with its entry, as data exports do (0034).
CREATE FUNCTION feedback_image_cleanup() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.image_key IS NOT NULL
       AND (TG_OP = 'DELETE' OR NEW.image_key IS DISTINCT FROM OLD.image_key) THEN
        INSERT INTO storage_cleanup (object_key, retain_until) VALUES (OLD.image_key, now())
            ON CONFLICT (object_key) DO UPDATE SET next_attempt = now();
    END IF;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END $$;
CREATE TRIGGER feedback_image_cleanup BEFORE DELETE OR UPDATE OF image_key ON feedback
    FOR EACH ROW EXECUTE FUNCTION feedback_image_cleanup();

-- Operator actions on feedback name their entry. No foreign key, as for the
-- other targets: the row outlives a deleted entry.
ALTER TABLE operator_audit ADD COLUMN target_feedback UUID;
