-- Replies (issue #251). `reply_to` names the answered message of the same
-- channel; the API checks that on insert. No foreign key on purpose: a
-- deleted original leaves the id dangling and the API reports it as
-- `{"deleted": true}`. `ON DELETE SET NULL` would erase that the row was a
-- reply at all, and every delete would rewrite the rows answering it.
-- Lookups go original-by-id (primary key), so no extra index.

ALTER TABLE messages ADD COLUMN reply_to UUID;
