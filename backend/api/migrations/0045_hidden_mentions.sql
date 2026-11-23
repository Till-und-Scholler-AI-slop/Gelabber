-- GEL-43: `@here` reaches invisible members (#590) too, but what others read
-- of the message must not show that they were connected.
--
-- `hidden_mentions`: the members among `mentions` whom only they themselves
-- see as mentioned (`messages/mentions.rs`). `mentions` stays everyone the
-- message mentions, for the unread counts and pushes.
--
-- Not backfilled: nothing recorded who was invisible when a message was
-- sent, so messages from before this migration keep everyone `@here`
-- reached in `mentions`, visible to all readers.
ALTER TABLE messages ADD COLUMN hidden_mentions UUID[] NOT NULL DEFAULT '{}';

-- `mentions` as `viewer` reads them: their own hidden mention, nobody
-- else's. A NULL viewer reads what everyone reads.
CREATE FUNCTION mentions_seen_by(mentions UUID[], hidden UUID[], viewer UUID)
RETURNS UUID[] LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN cardinality(hidden) = 0 THEN mentions ELSE
        ARRAY(SELECT u FROM unnest(mentions) WITH ORDINALITY AS t(u, n)
              WHERE u = viewer OR u <> ALL(hidden) ORDER BY n)
    END
$$;
