-- Creation order is immutable and allocated at INSERT, after the channel lock.
-- Unlike now(), it cannot precede the transaction that was just marked read.
-- REST catch-up uses (created_at,id), so stamp creation at INSERT too.
ALTER TABLE messages ALTER COLUMN created_at SET DEFAULT clock_timestamp();
CREATE SEQUENCE message_created_order_seq;
ALTER TABLE messages ADD COLUMN created_order BIGINT;
WITH ordered AS (
    SELECT id, row_number() OVER (ORDER BY created_at, id) AS ordinal FROM messages
)
UPDATE messages SET created_order=ordered.ordinal FROM ordered WHERE messages.id=ordered.id;
SELECT setval('message_created_order_seq', COALESCE(max(created_order), 1), count(*) > 0) FROM messages;
ALTER TABLE messages ALTER COLUMN created_order SET DEFAULT nextval('message_created_order_seq');
ALTER TABLE messages ALTER COLUMN created_order SET NOT NULL;
ALTER SEQUENCE message_created_order_seq OWNED BY messages.created_order;
CREATE INDEX messages_channel_created_order_idx ON messages(channel_id, created_order);

-- Keep the boundary when its message is deleted. A rejoin starts a new read scope.
CREATE TABLE channel_read_state (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    membership_at TIMESTAMPTZ NOT NULL,
    message_at TIMESTAMPTZ NOT NULL,
    created_order BIGINT NOT NULL,
    message_id UUID NOT NULL,
    PRIMARY KEY (user_id, channel_id)
);

CREATE INDEX messages_content_search_idx
    ON messages USING GIN (to_tsvector('simple', content));
