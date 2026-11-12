-- Per-account language (v0.10 "English"). Push, mail and both clients read it.
-- Existing accounts were German.
ALTER TABLE users
    ADD COLUMN locale TEXT NOT NULL DEFAULT 'de'
        CHECK (locale IN ('de', 'en'));
