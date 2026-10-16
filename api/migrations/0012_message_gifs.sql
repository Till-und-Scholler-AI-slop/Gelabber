-- GIF messages (v0.7). The provider's item as resolved by the API at send
-- time: provider, slug, title, dimensions and the media URLs the API proxy
-- streams (`/api/gifs/media`). NULL for every other message. A GIF stands
-- alone: its message has no text, whatever path writes it.

ALTER TABLE messages ADD COLUMN gif JSONB
    CONSTRAINT messages_gif_alone
    CHECK (gif IS NULL OR (jsonb_typeof(gif) = 'object' AND content = ''));
