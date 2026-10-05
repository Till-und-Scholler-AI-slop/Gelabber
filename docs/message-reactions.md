# Message reactions

Text channels and DMs share `PUT /api/messages/{id}/reactions/{emoji}` and
`DELETE /api/messages/{id}/reactions/{emoji}`. Percent-encode the UTF-8 emoji.
Both return the current full message. PUT requires the current send permission;
DELETE of the caller's own vote requires only current read access. Missing
messages and inaccessible chats return the existing JSON 404 contract. Invalid
emoji return `validation_failed` with `fields.emoji=invalid`.

The key `(message_id,user_id,emoji)` makes retries idempotent. Different emoji
from the same user are independent. A no-op preserves the message revision. A
changed vote allocates a revision and an `e` outbox event inside the existing
channel write lock. Content, `created_at` and `edited_at` do not change. History
reads aggregate votes in the same PostgreSQL statement snapshot as revision and
content. Deleting a message cascades votes. Reactions do not produce `c` events
or new-message notifications.

The lazy picker and API use the same hash-checked Unicode Emoji 18.0 corpus,
with German CLDR 48.2.3 annotations, search terms and categories. Sources and
Unicode License v3 are checked in under `shared/data/emoji-sources`; generated
copies must pass `python3 tools/generate-emoji-data.py --check`. No network emoji
service is used. Rendering follows the device's emoji font; custom server emoji
are deferred. Failed picker-module loading offers a page reload because failed
ES modules may remain cached for the lifetime of the document.

Optimistic state contains only one in-flight own vote, not an earlier message
snapshot. New edits and other people's votes remain canonical during a failed
click. Lower-revision replies cannot revive a delete or overwrite a newer edit.
Account-generation guards protect mutation submission, replies and recovery.

## Local acceptance

Run the normal API and web tests, then start a disposable migrated API,
PostgreSQL/Redis and Vite against it. The browser smoke deliberately creates
local fixture accounts, servers, messages and DMs:

```sh
GELABBER_REACTIONS_URL=http://127.0.0.1:5173 npm --prefix web run test:reactions-smoke
```

The smoke runs Chromium and Firefox and checks multiple votes, counters, own
highlighting, live edits/deletes, read-only removal, DM privacy, reload,
keyboard selection/Escape focus restoration and the picker at 320px. Reports
and screenshots go to `/tmp/gelabber-reactions-browser` (override with
`GELABBER_REACTIONS_OUTPUT`). This is browser/viewport acceptance, not a physical
mobile-device test. SQLx tests separately cover concurrent idempotent writes,
edit/delete races, outbox preservation through Redis failure and permission
loss while a request waits on the membership writer lock.
## Local upgrade and restore drill

`python3 tools/check-chat-migration-restore.py --container gelabber-chat-test-postgres --output /tmp/gelabber-chat-restore.json`
uses only fresh UUID databases in an explicitly named local test PostgreSQL
container. It upgrades the nine shipped migrations with existing messages,
then adds votes and a read cursor, makes a real custom-format `pg_dump` and
restores it with `pg_restore`. Ordered rows, full-text search, creation sequence,
read boundary and reaction cascades must survive. It checks the old writer's
SQL footprint after an additive upgrade; it does not claim an old application
binary, object-store backup or production deployment rollback was accepted.
