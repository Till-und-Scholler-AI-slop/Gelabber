// Posts or deletes a message as a fixture user, for scenes whose actions
// need someone else to write (new-message toasts). Both shot tools delete
// what a scene posted once its screenshot is taken, so the fixture stays
// as seeded.
// Run in web/:
//   node scripts/parity/say.mjs post <user> <channel-key> <text>   → prints the id
//   node scripts/parity/say.mjs delete <user> <message-id>
//   node scripts/parity/say.mjs revoke-unused <user> <server-key>   (invites a dialog made)
//   node scripts/parity/say.mjs unmute <user> <channel-key|server-key>  (notification settings)
// <channel-key> is "dm" or a channel name of fixture.json; "{lena}" (any
// user key) in the text becomes that user's mention token.
/* global process, console, fetch, URL */
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(await readFile(join(here, "fixture.json"), "utf8"));
const [command, who, target, ...words] = process.argv.slice(2);
const user = fixture.users[who];
if (!user || !["post", "delete", "revoke-unused", "unmute"].includes(command) || !target)
  throw new Error("usage: say.mjs post <user> <channel-key> <text> | delete <user> <id>");

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
  // "file=<file>" the text file scripts/parity/files/<file>.
  const attachment_ids = [];
  let text = words.join(" ");
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
  console.log(message.id);
} else if (command === "revoke-unused") {
  // Invite links a scene's dialog made: never used, made by this user.
  const server = fixture.servers[target]?.id;
  if (!server) throw new Error(`unknown server ${target}`);
  for (const invite of await call("GET", `/servers/${server}/invites`))
    if (invite.uses === 0 && invite.created_by === user.id)
      await call("DELETE", `/invites/${encodeURIComponent(invite.code)}`);
} else if (command === "unmute") {
  // Back to inheriting: no level of its own, not muted.
  const server = fixture.servers[target]?.id;
  const channel = target === "dm" ? fixture.dm : fixture.servers.home.channels[target];
  if (!server && !channel) throw new Error(`unknown channel or server ${target}`);
  const scope = server ? `/servers/${server}` : `/channels/${channel}`;
  await call("PUT", `${scope}/notification-settings`, { level: null, muted: false });
} else {
  await call("DELETE", `/messages/${target}`);
}
