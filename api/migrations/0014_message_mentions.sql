-- Mentions (issue #241). `mentions` holds the users a message names, resolved
-- when it is written: `<@user-id>` tokens of members and, for `@here`, the
-- members online at that moment. `@everyone` is a flag instead of a row per
-- member, so members who join later are not counted for it (their read scope
-- starts at joining anyway). Both are rewritten when the message is edited.
ALTER TABLE messages
    ADD COLUMN mentions UUID[] NOT NULL DEFAULT '{}',
    ADD COLUMN mentions_everyone BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX messages_mentions_idx ON messages USING GIN (mentions);
