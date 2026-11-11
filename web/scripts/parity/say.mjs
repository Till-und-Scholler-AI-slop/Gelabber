// Posts or deletes a message as a fixture user, for scenes whose actions
// need someone else to write (new-message toasts). Both shot tools delete
// what a scene posted once its screenshot is taken, so the fixture stays
// as seeded.
// Run in web/:
//   node scripts/parity/say.mjs post <user> <channel-key> <text>   → prints the id
//   node scripts/parity/say.mjs delete <user> <message-id>
//   node scripts/parity/say.mjs revoke-unused <user> <server-key>   (invites a dialog made)
//   node scripts/parity/say.mjs theme <user> <preset-id|theme JSON>  (reset.mjs undoes it)
//   node scripts/parity/say.mjs unmute <user> <channel-key|server-key>  (notification settings)
//   node scripts/parity/say.mjs invite-code <user> <server-key>  → the seeded invite's code
//   node scripts/parity/say.mjs oidc-pending   → the token of a first Google sign-in's name step
//   node scripts/parity/say.mjs oidc-link <user> [nopassword]  (Google connected; reset.mjs undoes it)
// <channel-key> is "dm" or a channel name of fixture.json; "{lena}" (any
// user key) in the text becomes that user's mention token.
/* global process, console, fetch, URL, setTimeout */
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(await readFile(join(here, "fixture.json"), "utf8"));
const [command, who, target, ...words] = process.argv.slice(2);

// Sign-in with Google, written straight into the database: the parity stack
// has a client id but never talks to Google.
if (command === "oidc-pending" || command === "oidc-link") {
  const psql = (process.env.GELABBER_PARITY_PSQL ??
    "docker exec -i gnative-pg psql -q -U gelabber -d gelabber -v ON_ERROR_STOP=1").split(" ");
  const run = (sql) =>
    execFileSync(psql[0], psql.slice(1), { input: sql, stdio: ["pipe", "inherit", "inherit"] });
  if (command === "oidc-pending") {
    // A new person's verified answer waiting for its name (/sign-in/name#token=…).
    const token = "0123456789abcdef".repeat(4);
    const hash = `sha256(convert_to('${token}', 'UTF8'))`;
    run(`
delete from oidc_results where code_hash = ${hash};
insert into oidc_results (code_hash, provider, subject, email, email_verified, name_hint,
                          purpose, binding_hash, signup, expires_at)
values (${hash}, 'google', 'parity-neu', 'neu@example.test', true, 'Nora Neu',
        'sign_in', sha256(convert_to('parity', 'UTF8')), true, now() + interval '30 minutes');
`);
    console.log(token);
  } else {
    const linked = fixture.users[who];
    if (!linked || (target !== undefined && target !== "nopassword"))
      throw new Error("usage: say.mjs oidc-link <user> [nopassword]");
    if (!/^[0-9a-f-]{36}$/.test(linked.id)) throw new Error(`not a uuid: ${linked.id}`);
    run(`
insert into user_identities (provider, subject, user_id, email)
values ('google', 'parity-${who}', '${linked.id}', '${linked.email}')
on conflict do nothing;
`);
    // "nopassword": Google only. The shot tools still sign in with the
    // password, so the hash goes once that sign-in has made its session.
    if (target === "nopassword")
      run(`
create or replace function parity_drop_password() returns trigger language plpgsql as $$
begin
  update users set password_hash = null where id = new.user_id;
  return new;
end $$;
set client_min_messages = warning;
drop trigger if exists parity_drop_password on sessions;
create trigger parity_drop_password after insert on sessions for each row
when (new.user_id = '${linked.id}') execute function parity_drop_password();
`);
  }
  process.exit(0);
}

const user = fixture.users[who];
const COMMANDS = ["post", "delete", "revoke-unused", "theme", "unmute", "invite-code"];
if (!user || !COMMANDS.includes(command) || !target)
  throw new Error(`usage: say.mjs ${COMMANDS.join("|")} <user> …`);

const cookies = new Map();
let csrf = null;
async function call(method, path, body) {
  const headers = { Accept: "application/json" };
  if (cookies.size) headers.Cookie = [...cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (method !== "GET" && csrf) headers["X-CSRF-Token"] = csrf;
  const res = await fetch(`${fixture.base}/api${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  for (const line of res.headers.getSetCookie()) {
    const [pair] = line.split(";");
    const at = pair.indexOf("=");
    cookies.set(pair.slice(0, at), pair.slice(at + 1));
  }
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} → ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

csrf = (await call("GET", "/auth/session")).csrf_token;
await call("POST", "/auth/login", { email: user.email, password: fixture.password });
csrf = (await call("GET", "/auth/session")).csrf_token;

if (command === "post") {
  const channel = target === "dm" ? fixture.dm : fixture.servers.home.channels[target];
  if (!channel) throw new Error(`unknown channel ${target}`);
  // A leading "image=<file>" attaches web/public/images/living-room/<file>,
  // "file=<file>" the text file scripts/parity/files/<file>. A leading
  // "ghost=<emoji>" reacts with it and then deletes the row in the database,
  // without an event: open clients keep showing a message the server no
  // longer has, so whatever they write to it fails.
  const attachment_ids = [];
  let text = words.join(" ");
  let ghost = null;
  if (text.startsWith("ghost=")) {
    const [spec, ...rest] = text.split(" ");
    ghost = spec.slice("ghost=".length);
    text = rest.join(" ");
  }
  if (text.startsWith("image=") || text.startsWith("file=")) {
    const [file, ...rest] = text.split(" ");
    text = rest.join(" ");
    const image = file.startsWith("image=");
    const filename = file.slice(file.indexOf("=") + 1);
    const data = await readFile(
      image
        ? join(here, "../../public/images/living-room", filename)
        : join(here, "files", filename),
    );
    const presigned = await call("POST", `/channels/${channel}/attachments`, {
      filename,
      content_type: image ? "image/png" : "text/plain",
      size: data.length,
    });
    const url = presigned.upload_url.startsWith("/")
      ? `${fixture.base}${presigned.upload_url}`
      : presigned.upload_url;
    const headers = { ...presigned.headers };
    if (presigned.upload_url.startsWith("/")) {
      headers.Cookie = [...cookies].map(([k, v]) => `${k}=${v}`).join("; ");
      headers["X-CSRF-Token"] = csrf;
    }
    const put = await fetch(url, { method: "PUT", headers, body: data });
    if (!put.ok) throw new Error(`upload → ${put.status} ${await put.text()}`);
    attachment_ids.push(presigned.id);
  }
  const content = text.replace(/\{(\w+)\}/g, (_, key) => `<@${fixture.users[key].id}>`);
  const message = await call("POST", `/channels/${channel}/messages`, {
    content,
    ...(attachment_ids.length ? { attachment_ids } : {}),
  });
  if (ghost) {
    await call("PUT", `/messages/${message.id}/reactions/${encodeURIComponent(ghost)}`);
    // Time for the open clients to show the reaction first.
    await new Promise((resolve) => setTimeout(resolve, 1000));
    if (!/^[0-9a-f-]{36}$/.test(message.id)) throw new Error(`not a uuid: ${message.id}`);
    const psql = (process.env.GELABBER_PARITY_PSQL ??
      "docker exec -i gnative-pg psql -q -U gelabber -d gelabber -v ON_ERROR_STOP=1").split(" ");
    execFileSync(psql[0], psql.slice(1), {
      input: `delete from messages where id = '${message.id}';`,
      stdio: ["pipe", "inherit", "inherit"],
    });
  }
  console.log(message.id);
} else if (command === "revoke-unused") {
  // Invite links a scene's dialog made: never used, made by this user.
  const server = fixture.servers[target]?.id;
  if (!server) throw new Error(`unknown server ${target}`);
  for (const invite of await call("GET", `/servers/${server}/invites`))
    if (invite.uses === 0 && invite.created_by === user.id)
      await call("DELETE", `/invites/${encodeURIComponent(invite.code)}`);
} else if (command === "invite-code") {
  // The server's oldest invite: the one seed.mjs made.
  const server = fixture.servers[target]?.id;
  if (!server) throw new Error(`unknown server ${target}`);
  const invites = await call("GET", `/servers/${server}/invites`);
  invites.sort((a, b) => a.created_at.localeCompare(b.created_at));
  if (!invites[0]) throw new Error(`no invite on ${target}`);
  console.log(invites[0].code);
} else if (command === "theme") {
  // A built-in id, or a whole custom theme (theme/model.ts ThemeDefinition)
  // that becomes the active one.
  const doc = await call("GET", "/me/themes");
  const spec = [target, ...words].join(" ");
  if (spec.startsWith("{")) {
    const theme = JSON.parse(spec);
    doc.customThemes = [...doc.customThemes.filter((t) => t.id !== theme.id), theme];
    doc.active = theme.id;
  } else {
    doc.active = spec;
  }
  await call("PUT", "/me/themes", doc);
} else if (command === "unmute") {
  // Back to inheriting: no level of its own, not muted.
  const server = fixture.servers[target]?.id;
  const channel = target === "dm" ? fixture.dm : fixture.servers.home.channels[target];
  if (!server && !channel) throw new Error(`unknown channel or server ${target}`);
  const scope = server ? `/servers/${server}` : `/channels/${channel}`;
  await call("PUT", `${scope}/notification-settings`, { level: null, muted: false });
} else {
  // A ghost (post ghost=…) is gone already.
  try {
    await call("DELETE", `/messages/${target}`);
  } catch (error) {
    if (!String(error.message).includes("→ 404")) throw error;
  }
}
