-- Sliding session lifetime (issue #195). Sessions now end after
-- `API_SESSION_IDLE_HOURS` (default 14 days) without use, and each use
-- pushes `expires_at` forward again, never past `created_at` plus
-- `API_SESSION_TTL_HOURS`. Rows from before this change carry the old fixed
-- 30-day expiry; cap them at the default idle window so a cookie that has
-- been lying around does not outlive the new rule. Renewal only ever moves
-- `expires_at` forward, so without this cap they would keep the old expiry.
UPDATE sessions SET expires_at = LEAST(expires_at, now() + interval '14 days');
