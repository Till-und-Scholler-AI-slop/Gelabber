-- Server icons: an uploaded image (`/api/server-icons/{server}/{file}`),
-- never a link, so viewers never contact a foreign host. NULL means none
-- and the client shows the name's initials. Same shape as
-- `servers::icon::parse_url`: two lower-case UUIDs.

ALTER TABLE servers
    ADD COLUMN icon_url TEXT,
    ADD CONSTRAINT servers_icon_url_uploaded
        CHECK (icon_url ~ '^/api/server-icons/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$');
