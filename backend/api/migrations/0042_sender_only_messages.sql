-- Silent sending into a DM closed by a block (#619).
--
-- When A blocks B, B can still write into their DM as if nothing changed:
-- the message is stored for B only. Only its author reads a sender-only
-- message: it is left out of the other member's history, search, unread and
-- mention counts and DM order, and no live event or push goes out for it.
-- Unblocking does not reveal it. Their reactions on the other member's
-- messages are sender-only the same way.
--
-- A constant default makes both a metadata-only change (Postgres 11+), so
-- they do not rewrite the tables. A rollback past this needs a backup.
ALTER TABLE messages ADD COLUMN sender_only BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE message_reactions ADD COLUMN sender_only BOOLEAN NOT NULL DEFAULT false;
