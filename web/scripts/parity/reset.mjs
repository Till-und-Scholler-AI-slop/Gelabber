// Puts the read state of the parity fixture back to the reference: Lena has
// read allgemein and nothing else, so spiele, musik and the DM stay unread.
// Opening a channel marks it read, so both shot tools call this before every
// scene; otherwise each run would change what the next one sees.
// Run in web/: node scripts/parity/reset.mjs
// GELABBER_PARITY_PSQL is the psql command line (default: the gnative-pg
// container of the local stack).
/* global process */
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
const allgemein = uuid(fixture.servers.home.channels.allgemein);
const sql = `
delete from channel_read_state where user_id = ${lena} and channel_id <> ${allgemein};
update channel_read_state r set message_id = m.id, message_at = m.created_at, created_order = m.created_order
from (select id, created_at, created_order from messages where channel_id = ${allgemein}
      order by created_order desc limit 1) m
where r.user_id = ${lena} and r.channel_id = ${allgemein};
`;
execFileSync(psql[0], psql.slice(1), { input: sql, stdio: ["pipe", "inherit", "inherit"] });
