-- Structured system messages (issue #314).
--
-- A message the server writes on its own (the "X is live in Y" notice) is
-- stored as a kind plus parameters instead of a finished German sentence,
-- so each client renders it in the reader's language
-- (`messages/system.rs`). Such a row keeps `content` empty; its author is
-- the person the notice is about.
--
-- Rows written before this migration stay plain text: an old notice cannot
-- be told apart from someone typing the same sentence.
ALTER TABLE messages
    ADD COLUMN system_kind TEXT CHECK (system_kind IN ('live_started')),
    ADD COLUMN system_params JSONB,
    ADD CONSTRAINT messages_system_check CHECK (
        (system_kind IS NULL AND system_params IS NULL)
        OR (system_kind IS NOT NULL AND jsonb_typeof(system_params) = 'object')
    );
