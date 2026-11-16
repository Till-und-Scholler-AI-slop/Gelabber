-- Alt text per picture (#575): written by the uploader when sending and
-- changed later through a message edit. NULL means none; the API trims it
-- and stores an empty text as NULL. It goes with the row, so deleting the
-- attachment or the account removes it.
ALTER TABLE attachments
    ADD COLUMN description TEXT
        CHECK (description IS NULL OR char_length(description) BETWEEN 1 AND 1000);
