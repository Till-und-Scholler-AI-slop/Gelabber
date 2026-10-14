-- GIF messages (v0.7). The provider's item as resolved by the API at send
-- time: provider, slug, title, dimensions and the media URLs the API proxy
-- streams (`/api/gifs/media`). NULL for every other message.

ALTER TABLE messages ADD COLUMN gif JSONB;
