-- Friends, friend codes and message requests (#563).
--
-- Who may write whom is decided by the API (`dms::relation`):
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

-- One row per (sender, recipient). Accepted requests are deleted. A
-- declined or withdrawn one stays for an hour from `closed_at`: after a
-- decline the sender may not ask again before that, after a withdrawal
-- asking again is quiet (no second push) until then.
CREATE TABLE friend_requests (
    sender_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    recipient_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status        TEXT        NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'declined', 'withdrawn')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at     TIMESTAMPTZ,
    PRIMARY KEY (sender_id, recipient_id),
    CHECK (sender_id <> recipient_id)
);
CREATE INDEX friend_requests_recipient_idx
    ON friend_requests (recipient_id) WHERE status = 'pending';

-- A DM is a message request while `dm_request_by` names who started it:
-- that person may send a few text messages, the other one accepts or
-- declines. NULL is an ordinary DM, so every DM that exists now counts as
-- accepted. `dm_request_sent` counts the messages sent into the request; a
-- deleted message still counts, so the limit and the one push hold.
-- `dm_declined_at` hides a declined request from the recipient; the sender
-- is not told and cannot tell it from one left unanswered.
ALTER TABLE channels
    ADD COLUMN dm_request_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN dm_request_sent SMALLINT NOT NULL DEFAULT 0,
    ADD COLUMN dm_declined_at  TIMESTAMPTZ,
    ADD CONSTRAINT channels_dm_request_only_dm
        CHECK (dm_request_by IS NULL OR kind = 'dm');
-- For the purge's ON DELETE SET NULL.
CREATE INDEX channels_dm_request_by_idx ON channels (dm_request_by)
    WHERE dm_request_by IS NOT NULL;
