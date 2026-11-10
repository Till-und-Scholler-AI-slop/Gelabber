-- Deleting a channel cascades into channel_read_state and attachments by
-- channel_id. Neither had an index that serves it: the read state's primary
-- key leads with user_id and the attachments index covers pending uploads
-- only, so every deleted channel (once per channel of a deleted server)
-- scanned both tables. Same shape as 0024.
CREATE INDEX channel_read_state_channel_idx ON channel_read_state (channel_id);
CREATE INDEX attachments_channel_id_idx ON attachments (channel_id);
