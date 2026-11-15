-- Friends, friend codes and message requests (#563).
--
-- Who may write whom is decided by the API (`dms/relation.rs`):
-- friends and accepted DMs write freely, people who share a server start a
-- message request, strangers can only send a friend request by code.

-- One code per account, `K7QX-M2PD`, generated on first use. Regenerating
-- replaces it, so the old one stops working at once.
ALTER TABLE users ADD COLUMN friend_code TEXT UNIQUE
    CHECK (friend_code ~ '^[A-HJ-NP-Z2-9]{4}-[A-HJ-NP-Z2-9]{4}$');

-- One row per pair, lower id first.
CREATE TABLE friendships (
    user_low    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    user_high   UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_low, user_high),
    CHECK (user_low < user_high)
);
CREATE INDEX friendships_high_idx ON friendships (user_high);

-- One row per (sender, recipient). Accepted and withdrawn requests are
-- deleted; a declined one stays as the 1 h cooldown and is replaced by a new
-- request after that.
CREATE TABLE friend_requests (
    sender_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    recipient_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status        TEXT        NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'declined')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    declined_at   TIMESTAMPTZ,
    PRIMARY KEY (sender_id, recipient_id),
    CHECK (sender_id <> recipient_id)
);
CREATE INDEX friend_requests_recipient_idx
    ON friend_requests (recipient_id) WHERE status = 'pending';

-- A DM is a message request while `dm_request_by` names who started it:
-- that person may send a few text messages, the other one accepts or
-- declines. NULL is an ordinary DM, so every DM that exists now counts as
-- accepted. `dm_request_since` starts the current round (the message limit
-- counts from it); `dm_declined_at` hides a declined request from the
-- recipient, and the sender may start a new round an hour later.
ALTER TABLE channels
    ADD COLUMN dm_request_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN dm_request_since TIMESTAMPTZ,
    ADD COLUMN dm_declined_at   TIMESTAMPTZ,
    ADD CONSTRAINT channels_dm_request_only_dm
        CHECK (dm_request_by IS NULL OR kind = 'dm');
