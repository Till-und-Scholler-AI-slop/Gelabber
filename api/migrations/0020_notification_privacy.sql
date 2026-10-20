-- Per-account notification privacy (issue #268). With it on, Web Push and the
-- page's own notifications say only who wrote ("Neue Nachricht von Ada"):
-- no preview and no conversation, so a locked screen shows nothing more.
ALTER TABLE users
    ADD COLUMN notifications_hide_content BOOLEAN NOT NULL DEFAULT false;
