-- Profile card customisation: what other members see about someone beyond
-- name and avatar. Every column is optional; NULL means "not set" and the
-- client falls back to its defaults. The API validates and normalises
-- (`api/src/auth/validate.rs`); these checks only keep any other write path
-- inside the same limits.

ALTER TABLE users
    ADD COLUMN bio               TEXT,
    ADD COLUMN pronouns          TEXT,
    ADD COLUMN status_text       TEXT,
    ADD COLUMN accent_color      TEXT,
    ADD COLUMN banner_color      TEXT,
    -- Uploaded banner only (`/api/banners/{user}/{file}`), never a link.
    ADD COLUMN banner_url        TEXT,
    ADD COLUMN avatar_decoration TEXT;

ALTER TABLE users
    ADD CONSTRAINT users_bio_length CHECK (char_length(bio) BETWEEN 1 AND 190),
    ADD CONSTRAINT users_pronouns_length CHECK (char_length(pronouns) BETWEEN 1 AND 40),
    ADD CONSTRAINT users_status_text_length CHECK (char_length(status_text) BETWEEN 1 AND 80),
    ADD CONSTRAINT users_accent_color_hex CHECK (accent_color ~ '^#[0-9a-f]{6}$'),
    ADD CONSTRAINT users_banner_color_hex CHECK (banner_color ~ '^#[0-9a-f]{6}$'),
    ADD CONSTRAINT users_banner_url_uploaded CHECK (banner_url LIKE '/api/banners/%'),
    ADD CONSTRAINT users_avatar_decoration_known
        CHECK (avatar_decoration IN ('ring', 'glow', 'rainbow', 'stars'));
