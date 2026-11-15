-- Manual presence (#590): Automatic, Away, Do not disturb or Invisible, for
-- the whole account, until `presence_mode_until` (none: until changed).
-- `auto` never has an end. The API sweeps modes that ran out back to `auto`.
ALTER TABLE users
    ADD COLUMN presence_mode TEXT NOT NULL DEFAULT 'auto'
        CHECK (presence_mode IN ('auto', 'idle', 'dnd', 'invisible')),
    ADD COLUMN presence_mode_until TIMESTAMPTZ,
    ADD CONSTRAINT users_presence_mode_until_check
        CHECK (presence_mode <> 'auto' OR presence_mode_until IS NULL);

CREATE INDEX users_presence_mode_until_idx ON users (presence_mode_until)
    WHERE presence_mode_until IS NOT NULL;
