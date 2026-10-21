-- Notification settings by scope (issue #294). The two forget triggers from
-- 0015 delete by (scope_kind, scope_id) alone, but the primary key leads
-- with user_id, so without this index every deleted server, and every
-- channel its deletion cascades to, has to search the primary key across
-- every user: a skip scan with one descent per user on PostgreSQL 18, a
-- full table scan before it. Every other query names the user first and
-- keeps using the primary key.
CREATE INDEX notification_settings_scope_idx ON notification_settings (scope_kind, scope_id);
