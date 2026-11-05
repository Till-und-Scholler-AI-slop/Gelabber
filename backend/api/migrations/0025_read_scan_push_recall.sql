-- Read sync and Web Push follow-ups (issue #277).
--
-- Without a read position since joining, the unread mention count walks the
-- mentioning messages since joining by time, so a channel quiet since then
-- costs one index probe instead of a scan of its history.
CREATE INDEX messages_channel_mentioning_at_idx ON messages (channel_id, created_at DESC)
    WHERE mentions_everyone OR mentions <> '{}';

-- Which subscriptions took a message's push, for as long as the push service
-- may still hold it (its TTL, an hour). Deleting the message sends each one
-- a replacement under the same topic (`push::recall`). No key to `messages`:
-- the rows are read after the message is gone, and age out on their own.
CREATE TABLE push_receipts (
    message_id UUID        NOT NULL,
    endpoint   TEXT        NOT NULL REFERENCES push_subscriptions (endpoint) ON DELETE CASCADE,
    sent_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, endpoint)
);

CREATE INDEX push_receipts_sent_at_idx ON push_receipts (sent_at);
CREATE INDEX push_receipts_endpoint_idx ON push_receipts (endpoint);
