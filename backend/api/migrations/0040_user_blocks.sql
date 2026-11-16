-- Person-to-person blocks (#565).
--
-- While A blocks B neither can open a DM with the other, write into their
-- DM, send a message request or a friend request (`crate::blocks`). B's
-- pushes and mentions do not reach A; clients fold B's server messages.
-- Blocking ends a friendship and the requests between the two.
CREATE TABLE user_blocks (
    blocker_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_id, blocked_id),
    CHECK (blocker_id <> blocked_id)
);
CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_id);
