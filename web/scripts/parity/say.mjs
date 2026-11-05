// Posts or deletes a message as a fixture user, for scenes whose actions
// need someone else to write (new-message toasts). Both shot tools delete
// what a scene posted once its screenshot is taken, so the fixture stays
// as seeded.
// Run in web/:
//   node scripts/parity/say.mjs post <user> <channel-key> <text>   → prints the id
//   node scripts/parity/say.mjs delete <user> <message-id>
// <channel-key> is "dm" or a channel name of fixture.json; "{lena}" (any
// user key) in the text becomes that user's mention token.
/* global process, console, fetch */
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(await readFile(join(here, "fixture.json"), "utf8"));
const [command, who, target, ...words] = process.argv.slice(2);
const user = fixture.users[who];
if (!user || !["post", "delete"].includes(command) || !target)
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
  const content = words
    .join(" ")
    .replace(/\{(\w+)\}/g, (_, key) => `<@${fixture.users[key].id}>`);
  const message = await call("POST", `/channels/${channel}/messages`, { content });
  console.log(message.id);
} else {
  await call("DELETE", `/messages/${target}`);
}
