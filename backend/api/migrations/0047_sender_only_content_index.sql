-- no-transaction
-- GEL-21: search reads B's own text (0046) for B: the few such rows of a
-- channel without a scan of all its messages.
--
-- Built CONCURRENTLY, so message writes go on while it reads the table;
-- that cannot run in a transaction, hence its own migration with one
-- statement. A build that fails leaves an INVALID index behind: drop it
-- (`DROP INDEX CONCURRENTLY messages_sender_only_content_idx`) before the
-- API starts again, or IF NOT EXISTS keeps the invalid one.
CREATE INDEX CONCURRENTLY IF NOT EXISTS messages_sender_only_content_idx
    ON messages (channel_id) WHERE sender_only_content IS NOT NULL;
