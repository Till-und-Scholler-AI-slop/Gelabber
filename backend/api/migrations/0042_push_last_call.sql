-- Issue #616: a session that runs out takes its push subscriptions with it.
-- Shortly before that, each of them gets one push that says so
-- (`push::last_call`). `warned_at` is when; a use of the session after it
-- arms the notice again.
ALTER TABLE push_subscriptions ADD COLUMN warned_at TIMESTAMPTZ;
