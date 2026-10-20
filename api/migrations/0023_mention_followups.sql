-- Mention follow-ups (issue #276).
--
-- `here_users`: whom a granted `@here` reached, named in the text or not, so
-- an edit that keeps `@here` but drops `<@X>` still mentions X when `@here`
-- reached them. NULL means no granted `@here`, or a message written before
-- this column; those are read from the text as before
-- (`messages/mentions.rs`, `Granted::of`).
ALTER TABLE messages ADD COLUMN here_users UUID[];

-- The unread mention count scans only messages that mention anybody, newest
-- first, instead of every unread message of the channel. Its query repeats
-- this predicate so the planner can use the index.
CREATE INDEX messages_channel_mentioning_idx ON messages (channel_id, created_order)
    WHERE mentions_everyone OR mentions <> '{}';
