-- Database revisions survive Redis resets. The outbox has no cascading FK:
-- committed delete/discovery notifications must survive domain row deletion.
CREATE SEQUENCE gateway_revision_seq AS BIGINT;
ALTER TABLE messages ADD COLUMN revision BIGINT NOT NULL DEFAULT 0;
CREATE TABLE gateway_outbox (
    id BIGINT PRIMARY KEY,
    channel_id UUID NOT NULL,
    server_id UUID NOT NULL,
    target_user UUID,
    kind TEXT NOT NULL CHECK (kind IN ('c','e','d','dm')),
    entity_id UUID,
    delta JSONB,
    next_attempt TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX gateway_outbox_channel_id_idx ON gateway_outbox (channel_id, id);
