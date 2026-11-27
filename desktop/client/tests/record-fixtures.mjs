// Records the API answers in fixtures/ from a running, seeded stack, for
// models.rs (GEL-376). The stack is the parity stack (desktop/README.md
// "Tests"): API, media server and Vite, seeded by web/scripts/parity/seed.mjs.
// What it writes happens in a scratch server, deleted at the end.
//
//   node desktop/client/tests/record-fixtures.mjs [base-url]          records
//   node desktop/client/tests/record-fixtures.mjs [base-url] --check  compares
//
// Record on a freshly seeded stack (unread_seeded is what the stack went
// through) whose API runs with the parity environment only: a GIF key,
// mail, TURN or other operators change gifs, mail, ticket and session.
//
// Recording writes fixtures/*.json and fixtures/recorded.json, with the
// recorder's tree and each file's hash: models.rs fails on a fixture that
// differs from its hash, a guard against stray edits, not a lock. --check
// records into a scratch folder and compares keys and value types, not
// values: it fails when the API no longer sends a key or type a fixture
// holds, and warns when it sends a new one. e2e-extended.yml runs it after
// merges.
/* global process, console, fetch, WebSocket, URL, AbortSignal, setTimeout, clearTimeout */
import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname;
const committed = join(here, "fixtures");
const args = process.argv.slice(2);
const check = args.includes("--check");
const BASE = args.find((a) => !a.startsWith("--")) ?? "http://127.0.0.1:5180";
// What the stack does not answer within this ends the recording by name.
const LIMIT_MS = 20_000;

/** `promise`, or an error naming `what` after LIMIT_MS. */
function within(what, promise) {
  let timer;
  const late = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`no answer: ${what}`)), LIMIT_MS);
  });
  return Promise.race([promise, late]).finally(() => clearTimeout(timer));
}

const seeded = JSON.parse(
  await readFile(
    process.env.GELABBER_FIXTURE ??
      join(here, "../../../web/scripts/parity/fixture.json"),
    "utf8",
  ),
);

class Person {
  constructor(handle) {
    this.handle = handle;
    this.cookies = new Map();
    this.csrf = null;
  }

  async call(method, path, body) {
    const headers = { Accept: "application/json" };
    if (this.cookies.size) headers.Cookie = this.cookie();
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (method !== "GET" && this.csrf) headers["X-CSRF-Token"] = this.csrf;
    const res = await fetch(`${BASE}/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "manual",
      signal: AbortSignal.timeout(LIMIT_MS),
    });
    for (const line of res.headers.getSetCookie()) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      this.cookies.set(pair.slice(0, at), pair.slice(at + 1));
    }
    const text = await res.text();
    if (!res.ok)
      throw new Error(
        `${this.handle}: ${method} ${path} → ${res.status} ${text}`,
      );
    return text ? JSON.parse(text) : null;
  }

  cookie() {
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  }

  async signIn() {
    this.csrf = (await this.call("GET", "/auth/session")).csrf_token;
    await this.call("POST", "/auth/login", {
      email: seeded.users[this.handle].email,
      password: seeded.password,
    });
    const session = await this.call("GET", "/auth/session");
    this.csrf = session.csrf_token;
    return session;
  }
}

/**
 * The gateway socket of a person: every frame it receives, and in `kept`
 * the ones waited for, in order. Only those are recorded: the others (a
 * room snapshot now and then, one's own presence) come or not by timing.
 */
class Socket {
  constructor(person) {
    this.frames = [];
    this.kept = [];
    this.waiters = [];
    const url = new URL("/ws", BASE);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    this.ws = new WebSocket(url, { headers: { Cookie: person.cookie() } });
    this.ws.addEventListener("message", (event) => {
      const frame = JSON.parse(event.data);
      this.frames.push(frame);
      this.waiters = this.waiters.filter((w) => !w(frame));
    });
    this.opened = within(
      `${person.handle}'s gateway`,
      new Promise((resolve, reject) => {
        this.ws.addEventListener("open", resolve, { once: true });
        this.ws.addEventListener("error", reject, { once: true });
      }),
    );
  }

  send(frame) {
    this.ws.send(JSON.stringify(frame));
  }

  /** The next frame (from now on, or already received when `seen`) that `match`es. */
  async until(what, match, options) {
    const frame = await this.next(what, match, options);
    if (!this.kept.includes(frame)) this.kept.push(frame);
    return frame;
  }

  next(what, match, { seen = false } = {}) {
    if (seen) {
      const found = this.frames.find(match);
      if (found) return Promise.resolve(found);
    }
    return new Promise((resolve, reject) => {
      // A frame that never comes ends the recording with its name.
      const timer = setTimeout(
        () =>
          reject(
            new Error(
              `no frame: ${what}; received ${JSON.stringify(this.frames)}`,
            ),
          ),
        LIMIT_MS,
      );
      this.waiters.push((frame) => {
        if (!match(frame)) return false;
        clearTimeout(timer);
        resolve(frame);
        return true;
      });
    });
  }

  close() {
    const closed = within(
      "the gateway's close",
      new Promise((resolve) =>
        this.ws.addEventListener("close", resolve, { once: true }),
      ),
    );
    this.ws.close();
    return closed;
  }
}

async function record(out) {
  const recorded = {};
  const keep = (name, value) => {
    recorded[name] = value;
    return value;
  };
  const lena = new Person("lena");
  const jonas = new Person("jonas");
  keep("session", await lena.signIn());
  await jonas.signIn();
  const home = seeded.servers.home;

  // Account and settings, as the app reads them at start.
  keep("me", await lena.call("GET", "/me"));
  keep(
    "user_profile",
    await lena.call("GET", `/users/${seeded.users.jonas.id}`),
  );
  keep("mail", await lena.call("GET", "/auth/mail"));
  keep("themes", await lena.call("GET", "/me/themes"));
  keep("notification_privacy", await lena.call("GET", "/notification-privacy"));
  keep("gifs", await lena.call("GET", "/gifs"));
  keep("unread_seeded", await lena.call("GET", "/messages/unread"));

  // The seeded workspace.
  keep("servers", await lena.call("GET", "/servers"));
  keep("server_wohnzimmer", await lena.call("GET", `/servers/${home.id}`));
  keep(
    "messages_allgemein",
    await lena.call(
      "GET",
      `/channels/${home.channels.allgemein}/messages?limit=50`,
    ),
  );
  keep("dms", await lena.call("GET", "/dms"));
  keep(
    "dm",
    await lena.call("POST", "/dms", { user_id: seeded.users.jonas.id }),
  );
  keep(
    "messages_dm",
    await lena.call("GET", `/channels/${seeded.dm}/messages?limit=50`),
  );
  keep(
    "ticket",
    await lena.call("POST", `/channels/${home.channels.Sofa}/media-ticket`),
  );

  // A scratch server for everything that writes.
  const created = keep(
    "server_create",
    await lena.call("POST", "/servers", { name: "Aufnahme" }),
  );
  const scratch = created.id;
  try {
    const text = created.channels.find((c) => c.kind === "text").id;
    const voice = created.channels.find((c) => c.kind === "voice").id;
    const invite = keep(
      "invite",
      // With a limit and an expiry, so both round-trip as values.
      await lena.call("POST", `/servers/${scratch}/invites`, {
        max_uses: 5,
        expires_in_hours: 24,
      }),
    );
    keep("invites", await lena.call("GET", `/servers/${scratch}/invites`));
    keep("invite_preview", await jonas.call("GET", `/invites/${invite.code}`));
    keep("join", await jonas.call("POST", `/invites/${invite.code}/join`));
    keep("server_detail", await lena.call("GET", `/servers/${scratch}`));
    keep(
      "presign",
      await lena.call("POST", `/channels/${text}/attachments`, {
        filename: "hallo.txt",
        content_type: "text/plain",
        size: 5,
      }),
    );

    // Lena's gateway while Jonas comes, types, joins voice, writes and goes.
    const socket = new Socket(lena);
    await socket.opened;
    socket.send({ op: "s", s: scratch });
    await socket.until(
      "server subscribed",
      (f) => f.op === "ok" && f.s === scratch && !f.c,
      { seen: true },
    );
    await socket.until(
      "the server's presence",
      (f) => f.op === "p" && f.s === scratch && Array.isArray(f.snap),
      { seen: true },
    );
    await socket.until(
      "the server's voice rooms",
      (f) => f.op === "sig" && f.t === "r" && f.s === scratch,
      { seen: true },
    );
    socket.send({ op: "s", s: scratch, c: text });
    await socket.until(
      "channel subscribed",
      (f) => f.op === "ok" && f.c === text,
      { seen: true },
    );
    const jonasSocket = new Socket(jonas);
    await jonasSocket.opened;
    jonasSocket.send({ op: "s", s: scratch, c: text });
    await jonasSocket.until(
      "Jonas subscribed",
      (f) => f.op === "ok" && f.c === text,
      { seen: true },
    );
    await socket.until(
      "Jonas online",
      (f) => f.op === "p" && f.u === seeded.users.jonas.id && f.st === "o",
      { seen: true },
    );
    jonasSocket.send({ op: "y", s: scratch, c: text, on: true });
    await socket.until("Jonas types", (f) => f.op === "y" && f.on === true, {
      seen: true,
    });
    jonasSocket.send({ op: "sig", s: scratch, c: voice, t: "j" });
    await socket.until("Jonas in voice", (f) => f.op === "sig" && f.t === "j", {
      seen: true,
    });
    jonasSocket.send({ op: "sig", s: scratch, c: voice, t: "m", on: true });
    await socket.until("Jonas muted", (f) => f.op === "sig" && f.t === "m", {
      seen: true,
    });

    const first = keep(
      "message_create",
      await lena.call("POST", `/channels/${text}/messages`, {
        content: "Erste **Nachricht** ||geheim||",
        nonce: randomUUID(),
      }),
    );
    const reply = keep(
      "message_reply",
      await jonas.call("POST", `/channels/${text}/messages`, {
        content: `<@${seeded.users.lena.id}> Antwort`,
        reply_to: first.id,
      }),
    );
    for (const [what, message] of [
      ["Lena's message", first],
      ["Jonas' reply", reply],
    ])
      await socket.until(
        what,
        (f) => f.op === "e" && f.t === "c" && f.i === message.id,
        { seen: true },
      );
    keep(
      "reaction",
      await jonas.call(
        "PUT",
        `/messages/${first.id}/reactions/${encodeURIComponent("👍")}`,
      ),
    );
    keep(
      "edit",
      await lena.call("PATCH", `/messages/${first.id}`, {
        content: "Erste **Nachricht**, geändert",
      }),
    );
    await socket.until(
      "the edit",
      (f) => f.op === "e" && f.t === "e" && f.r === recorded.edit.revision,
      { seen: true },
    );
    keep(
      "search",
      await lena.call("GET", `/channels/${text}/messages/search?q=Nachricht`),
    );
    keep(
      "context",
      await lena.call("GET", `/channels/${text}/messages/${reply.id}/context`),
    );
    keep(
      "read",
      await lena.call("PUT", `/channels/${text}/read`, {
        message_id: reply.id,
      }),
    );
    await socket.until("read state", (f) => f.op === "rd", { seen: true });
    keep("unread", await lena.call("GET", "/messages/unread"));
    keep(
      "ns_set",
      await lena.call("PUT", `/channels/${text}/notification-settings`, {
        level: "mentions",
        muted: true,
        mute_minutes: 60,
      }),
    );
    await socket.until("notification settings", (f) => f.op === "ns", {
      seen: true,
    });
    keep("ns_list", await lena.call("GET", "/notification-settings"));
    await jonas.call("DELETE", `/messages/${reply.id}`);
    await socket.until(
      "the deletion",
      (f) => f.op === "e" && f.t === "d" && f.c === text,
      { seen: true },
    );
    jonasSocket.send({ op: "y", s: scratch, c: text, on: false });
    await socket.until(
      "Jonas stops typing",
      (f) => f.op === "y" && f.on === false,
      { seen: true },
    );
    jonasSocket.send({ op: "sig", s: scratch, c: voice, t: "l" });
    await socket.until(
      "Jonas leaves voice",
      (f) => f.op === "sig" && f.t === "l",
      { seen: true },
    );
    await jonasSocket.close();
    await socket.until(
      "Jonas offline",
      (f) => f.op === "p" && f.u === seeded.users.jonas.id && f.st === "x",
      { seen: true },
    );
    await lena.call("POST", `/servers/${scratch}/kick`, {
      user_id: seeded.users.jonas.id,
    });
    await socket.until(
      "Jonas removed",
      (f) => f.op === "e" && f.t === "d" && !f.c,
      { seen: true },
    );
    await lena.call("PUT", "/me/locale", { locale: "en" });
    await socket.until("locale", (f) => f.op === "lc", { seen: true });
    await lena.call("PUT", "/me/locale", { locale: "de" });
    await socket.close();
    keep("frames", socket.kept);
  } finally {
    await lena
      .call("DELETE", `/servers/${scratch}`)
      .catch((error) => console.error(String(error)));
  }

  for (const [name, value] of Object.entries(recorded))
    await writeFile(
      join(out, `${name}.json`),
      `${JSON.stringify(value, null, 2)}\n`,
    );
  return recorded;
}

/**
 * Keys and value types, not values: what the client's models depend on.
 * Every path with the type found there (`.messages[].author.name string`);
 * the elements of an array count together, whatever their number.
 */
function shape(value, path = "", out = new Set()) {
  if (Array.isArray(value)) {
    out.add(`${path} array`);
    for (const element of value) shape(element, `${path}[]`, out);
  } else if (value && typeof value === "object") {
    out.add(`${path} object`);
    for (const [key, child] of Object.entries(value))
      shape(child, `${path}.${key}`, out);
  } else {
    out.add(`${path} ${value === null ? "null" : typeof value}`);
  }
  return out;
}

/** FNV-1a, 64 bits, as hex: models.rs computes the same without a crate. */
function fnv1a(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes)
    hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return hash.toString(16).padStart(16, "0");
}

if (check) {
  const fresh = await mkdtemp(join(tmpdir(), "gelabber-fixtures-"));
  try {
    await record(fresh);
    const differ = [];
    const details = [];
    for (const file of (await readdir(committed)).filter(
      (f) => f.endsWith(".json") && f !== "recorded.json",
    )) {
      const now = JSON.parse(
        await readFile(join(fresh, file), "utf8").catch(() => "null"),
      );
      const then = JSON.parse(await readFile(join(committed, file), "utf8"));
      const [a, b] = [shape(then), shape(now)];
      const gone = [...a].filter((p) => !b.has(p));
      const added = [...b].filter((p) => !a.has(p));
      // A client that allows unknown keys reads an addition; a key or type
      // gone (a type change is one gone and one added) breaks it.
      if (gone.length) {
        differ.push(file);
        details.push(
          `${file}: the API no longer sends ${JSON.stringify(gone.slice(0, 8))}`,
        );
      }
      if (added.length)
        console.log(
          `::warning::${file}: the API sends what the fixture lacks: ${JSON.stringify(added.slice(0, 8))}; record again`,
        );
    }
    if (differ.length) {
      console.error(
        `The API sends other shapes than these fixtures: ${differ.join(", ")}. ` +
          "Record them again (node desktop/client/tests/record-fixtures.mjs) and adapt models.rs.\n" +
          details.join("\n"),
      );
      process.exit(1);
    }
    console.log("every fixture has the shape the API sends");
  } finally {
    await rm(fresh, { recursive: true, force: true });
  }
} else {
  // Before the fixtures change: the recorder's own tree.
  const describe = execFileSync(
    "git",
    ["describe", "--tags", "--always", "--dirty"],
    {
      cwd: here,
      encoding: "utf8",
    },
  ).trim();
  const fresh = await mkdtemp(join(tmpdir(), "gelabber-fixtures-"));
  try {
    await record(fresh);
    for (const file of (await readdir(committed)).filter((f) =>
      f.endsWith(".json"),
    ))
      await rm(join(committed, file));
    for (const file of await readdir(fresh))
      await writeFile(join(committed, file), await readFile(join(fresh, file)));
  } finally {
    await rm(fresh, { recursive: true, force: true });
  }
  const files = {};
  for (const file of (await readdir(committed))
    .filter((f) => f.endsWith(".json"))
    .sort())
    files[file] = fnv1a(await readFile(join(committed, file)));
  // The tree of this script; the API's build is whatever served BASE.
  const manifest = { recorder_tree: describe, files };
  await writeFile(
    join(committed, "recorded.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  console.log(
    `recorded ${Object.keys(files).length} fixtures with the recorder of ${describe}`,
  );
}
