// Puts the read state of the parity fixture back to the reference: Lena has
// read allgemein and nothing else, so spiele, musik and the DM stay unread.
// Opening a channel marks it read, so both shot tools call this before every
// scene; otherwise each run would change what the next one sees. Themes
// (say.mjs theme), notification settings, the language (a scene's
// `locale`) and Google sign-in (say.mjs oidc-*) go back to the seeded ones too,
// Lena's and Kai's signed-in devices to the reference's fixed ones, and no
// fixture account has a data export (say.mjs export). Feedback goes back to the
// seeded entries and threads: anything a scene added is deleted, and Tom's
// reply is unread again.
// Run in web/: node scripts/parity/reset.mjs
// GELABBER_PARITY_PSQL is the psql command line (default: the gnative-pg
// container of the local stack).
/* global process, URL */
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(readFileSync(join(here, "fixture.json"), "utf8"));
const psql = (process.env.GELABBER_PARITY_PSQL ??
  "docker exec -i gnative-pg psql -q -U gelabber -d gelabber -v ON_ERROR_STOP=1").split(" ");
const uuid = (v) => {
  if (!/^[0-9a-f-]{36}$/.test(v)) throw new Error(`not a uuid: ${v}`);
  return `'${v}'`;
};
const lena = uuid(fixture.users.lena.id);
const mira = uuid(fixture.users.mira.id);
const tom = uuid(fixture.users.tom.id);
const kai = uuid(fixture.users.kai.id);
const jonas = uuid(fixture.users.jonas.id);
const allgemein = uuid(fixture.servers.home.channels.allgemein);
const people = Object.values(fixture.users)
  .map((u) => uuid(u.id))
  .join(", ");
// The document of the seeded accounts: "Gelabber Dunkel", no own themes.
const seededTheme = `'{"active": "dark", "version": 1, "revision": 1, "customThemes": []}'::jsonb`;
const sql = `
update account_themes set revision = 1, document = ${seededTheme}, updated_at = now()
where user_id in (${people})
  and (document->>'active' <> 'dark' or document->'customThemes' <> '[]'::jsonb);
delete from notification_settings where user_id in (${people});
update users set notifications_hide_content = false
where id in (${people}) and notifications_hide_content;
update users set locale = 'de' where id in (${people}) and locale <> 'de';
delete from channel_read_state where user_id = ${lena} and channel_id <> ${allgemein};
update channel_read_state r set message_id = m.id, message_at = m.created_at, created_order = m.created_order
from (select id, created_at, created_order from messages where channel_id = ${allgemein}
      order by created_order desc limit 1) m
where r.user_id = ${lena} and r.channel_id = ${allgemein};
set client_min_messages = warning;
drop trigger if exists parity_drop_password on sessions;
drop function if exists parity_drop_password();
-- Every fixture account has the same password, so another's hash restores it.
update users set password_hash = (select password_hash from users
                                  where id in (${people}) and password_hash is not null limit 1)
where id in (${people}) and password_hash is null;
delete from user_identities where user_id in (${people});
delete from oidc_results where subject = 'parity-neu';
-- Signed-in devices: Lena's two other devices of the reference, and nothing
-- else; the scene's own sign-in adds the current one.
delete from sessions where user_id in (${lena}, ${kai});
insert into sessions (token_hash, user_id, created_at, last_used_at, expires_at, device_client, device_os)
values (sha256(convert_to('parity-lena-firefox', 'UTF8')), ${lena}, now() - interval '1 day',
        now() - interval '1 day', now() + interval '14 days', 'firefox', 'windows'),
       (sha256(convert_to('parity-lena-desktop', 'UTF8')), ${lena}, now() - interval '12 days',
        now() - interval '12 days', now() + interval '14 days', 'desktop', 'windows');
-- A data export (say.mjs export); the trigger queues its archive key.
delete from account_exports where user_id in (${people});
-- Feedback (seed.mjs): Tom's problem with Lena's one reply, unread; Jonas'
-- idea; Mira's praise, done. Anything else a scene added goes.
delete from feedback where not ((user_id = ${tom} and kind = 'problem')
                             or (user_id = ${jonas} and kind = 'idea')
                             or (user_id = ${mira} and kind = 'praise'));
delete from feedback_messages m using feedback f
where m.feedback_id = f.id
  and (f.user_id <> ${tom}
       or m.id <> (select id from feedback_messages where feedback_id = f.id
                   order by created_at, id limit 1));
update feedback f set last_activity_at = coalesce(
  (select max(created_at) from feedback_messages where feedback_id = f.id), f.created_at);
update feedback f set status = 'open', done_at = null, done_by = null,
  person_unread_since = (select min(created_at) from feedback_messages
                         where feedback_id = f.id and author_kind = 'operator')
where user_id in (${tom}, ${jonas});
update feedback set status = 'done', done_at = coalesce(done_at, last_activity_at),
  done_by = coalesce(done_by, ${lena}), person_unread_since = null
where user_id = ${mira};
`;
execFileSync(psql[0], psql.slice(1), { input: sql, stdio: ["pipe", "inherit", "inherit"] });
