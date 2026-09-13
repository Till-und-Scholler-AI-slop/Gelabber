-- 1:1 DMs are a channel kind with exactly two participants (issue #9).
-- They sit in `channels` so message REST and the WS topic stay the same.
-- No server, no category, no group DM. The pair key is the sorted user
-- ids so A→B and B→A resolve to one row.

ALTER TABLE channels ALTER COLUMN server_id DROP NOT NULL;

ALTER TABLE channels DROP CONSTRAINT channels_kind_check;
ALTER TABLE channels ADD CONSTRAINT channels_kind_check
    CHECK (kind IN ('text', 'voice', 'dm'));

ALTER TABLE channels ADD COLUMN pair_key TEXT;

CREATE UNIQUE INDEX channels_dm_pair_key_uidx
    ON channels (pair_key) WHERE kind = 'dm';

ALTER TABLE channels ADD CONSTRAINT channels_dm_shape CHECK (
    (kind = 'dm'
        AND server_id IS NULL
        AND category_id IS NULL
        AND pair_key IS NOT NULL)
    OR (kind IN ('text', 'voice')
        AND server_id IS NOT NULL
        AND pair_key IS NULL)
);

CREATE TABLE channel_members (
    channel_id UUID        NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

CREATE INDEX channel_members_user_id_idx ON channel_members (user_id, joined_at);
