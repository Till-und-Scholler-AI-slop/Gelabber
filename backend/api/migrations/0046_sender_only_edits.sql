-- GEL-21: the one blocked edits a message the blocker has already seen.
--
-- While A blocks B, B's new messages are sender-only (#619, 0042). An edit
-- of a message A saw before the block is stored for B alone as well: A
-- keeps the text and alt texts A saw, also after an unblock, and B reads
-- B's own. An edit after the unblock is everyone's again and drops B's
-- own text (and the alt texts it sets).
--
-- `sender_only_content`: B's text, NULL while B reads the shared one.
-- `sender_only_edited_at`: when B last changed it.
-- `sender_only_descriptions`: B's alt texts by attachment id (a JSON null
-- clears one), NULL while B reads the shared ones.
-- `sender_only_revision`: the revision of B's last own edit, so B's
-- clients take it over what they held.
--
-- Nullable without a default: a metadata-only change, no table rewrite.
ALTER TABLE messages
    ADD COLUMN sender_only_content TEXT,
    ADD COLUMN sender_only_edited_at TIMESTAMPTZ,
    ADD COLUMN sender_only_descriptions JSONB,
    ADD COLUMN sender_only_revision BIGINT;

-- Search reads B's own text for B: the few such rows of a channel without
-- a scan of all its messages.
CREATE INDEX messages_sender_only_content_idx ON messages (channel_id)
    WHERE sender_only_content IS NOT NULL;
