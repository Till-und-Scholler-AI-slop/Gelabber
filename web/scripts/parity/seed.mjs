// Seeds a fresh local stack with the fixed data both clients are compared on
// (desktop/parity): people, two servers, channels, messages, a DM, and
// roles with a channel limited to them, and, when the API runs with
// API_OPERATOR_EMAILS=lena@example.test (desktop/README.md "Parity"), beta
// invitations and a report for the "Betrieb" scenes. The directory
// "Entdecken" needs API_DIRECTORY=true: listings by Ada, Mira, Tom, Jonas and
// Lena, and Kai's request to join Lena's Wohnzimmer.
// Run in web/: node scripts/parity/seed.mjs [base-url] [out.json]
// The base URL is the web origin (Vite proxies /api), default 127.0.0.1:5180.
/* global process, console, fetch */
import { writeFile } from "node:fs/promises";

const BASE = process.argv[2] ?? "http://127.0.0.1:5180";
const OUT = process.argv[3] ?? "scripts/parity/fixture.json";
const PASSWORD = "parity-pass-123";

const PEOPLE = [
  ["lena", "Lena"],
  ["jonas", "Jonas"],
  ["mira", "Mira"],
  ["tom", "Tom"],
  ["ada", "Ada"],
  ["kai", "Kai"],
];

class Client {
  constructor(name) {
    this.name = name;
    this.cookies = new Map();
    this.csrf = null;
  }

  async call(method, path, body) {
    const headers = { Accept: "application/json" };
    if (this.cookies.size)
      headers.Cookie = [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (method !== "GET" && this.csrf) headers["X-CSRF-Token"] = this.csrf;
    const res = await fetch(`${BASE}/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "manual",
    });
    for (const line of res.headers.getSetCookie()) {
      const [pair] = line.split(";");
      const at = pair.indexOf("=");
      this.cookies.set(pair.slice(0, at), pair.slice(at + 1));
    }
    const text = await res.text();
    const data = text ? JSON.parse(text) : null;
    if (!res.ok)
      throw new Error(`${this.name}: ${method} ${path} → ${res.status} ${text}`);
    return data;
  }

  async refreshCsrf() {
    const session = await this.call("GET", "/auth/session");
    this.csrf = session.csrf_token;
    return session;
  }

  async signIn(handle, name) {
    await this.refreshCsrf();
    const email = `${handle}@example.test`;
    try {
      await this.call("POST", "/auth/register", { email, password: PASSWORD, name });
    } catch (error) {
      if (!String(error).includes(" 409 ")) throw error;
      await this.call("POST", "/auth/login", { email, password: PASSWORD });
    }
    const session = await this.refreshCsrf();
    this.user = session.user;
    return this;
  }
}

const say = (client, channel, content, extra = {}) =>
  client.call("POST", `/channels/${channel}/messages`, { content, ...extra });

const people = {};
for (const [handle, name] of PEOPLE)
  people[handle] = await new Client(name).signIn(handle, name);
const { lena, jonas, mira, tom, ada, kai } = people;

// The discover scenes need the directory (v0.10); without it the seed would
// stop halfway, so it stops here.
const directory = await lena.call("GET", "/directory");
if (!directory.enabled) {
  console.error(
    "error: the directory is off; run the parity stack's API with " +
      "API_DIRECTORY=true (desktop/README.md \"Parity\") and recreate the database",
  );
  process.exit(1);
}

const existing = await lena.call("GET", "/servers");
if (existing.some((s) => s.name === "Wohnzimmer")) {
  console.log("already seeded; recreate the database to seed again");
  process.exit(0);
}

// Wohnzimmer: Lena's server, everyone joins.
const home = await lena.call("POST", "/servers", { name: "Wohnzimmer" });
const homeDetail = await lena.call("GET", `/servers/${home.id}`);
const general = homeDetail.channels.find((c) => c.kind === "text");
const textCat = await lena.call("POST", `/servers/${home.id}/categories`, { name: "Plaudern" });
// A new server comes with a "Sprachkanäle" category; Sofa and Kino join it
// rather than a second category of the same name.
const voiceCat = homeDetail.categories.find((c) => c.name === "Sprachkanäle");
const games = await lena.call("POST", `/servers/${home.id}/channels`, {
  name: "spiele", kind: "text", category_id: textCat.id,
});
const music = await lena.call("POST", `/servers/${home.id}/channels`, {
  name: "musik", kind: "text", category_id: textCat.id,
});
const sofa = await lena.call("POST", `/servers/${home.id}/channels`, {
  name: "Sofa", kind: "voice", category_id: voiceCat.id,
});
const cinema = await lena.call("POST", `/servers/${home.id}/channels`, {
  name: "Kino", kind: "voice", category_id: voiceCat.id,
});
const invite = await lena.call("POST", `/servers/${home.id}/invites`, {});
for (const p of [jonas, mira, tom, ada])
  await p.call("POST", `/invites/${invite.code}/join`);

// Spieleabend: Jonas' server, Lena and Tom join.
const evening = await jonas.call("POST", "/servers", { name: "Spieleabend" });
const eveningInvite = await jonas.call("POST", `/servers/${evening.id}/invites`, {});
for (const p of [lena, tom])
  await p.call("POST", `/invites/${eveningInvite.code}/join`);

// allgemein: a conversation that shows grouping, markdown, a mention, a
// reply, a code block, emoji and reactions.
const g = general.id;
await say(jonas, g, "Hallo zusammen! 👋");
await say(jonas, g, "Wer ist heute Abend dabei?");
const plan = await say(mira, g, "Ich bin dabei. **Film oder Spiel?**");
await say(tom, g, `Spiel! Ich hätte Lust auf *Brettspiele* oder etwas Kooperatives.`);
await say(lena, g, `Klingt gut, @Jonas bringst du die Snacks mit?`);
await say(jonas, g, "Klar, ich bring Chips und Limo mit 🍿", { reply_to: plan.id });
await say(ada, g, "Ich habe die Regeln schon mal rausgesucht:\n> Jede Runde zieht jede Person zwei Karten.\n> Wer zuerst zehn Punkte hat, gewinnt.");
const code = await say(tom, g, "Für die Punkte habe ich ein kleines Skript:\n```\nfn punkte(karten: &[u8]) -> u32 {\n    karten.iter().map(|k| *k as u32).sum()\n}\n```");
await say(mira, g, "Perfekt, dann treffen wir uns um `20:00` im Sofa.");
await say(lena, g, "Bis später! 🎲");
await jonas.call("PUT", `/messages/${code.id}/reactions/${encodeURIComponent("👍")}`);
await mira.call("PUT", `/messages/${code.id}/reactions/${encodeURIComponent("👍")}`);
await lena.call("PUT", `/messages/${code.id}/reactions/${encodeURIComponent("🎉")}`);

await say(tom, games.id, "Neue Runde am Freitag?");
await say(ada, games.id, "Gerne, ab 19 Uhr.");
await say(jonas, music.id, "Hört ihr gerade was Gutes?");

// A DM from Jonas to Lena.
const dm = await jonas.call("POST", "/dms", { user_id: lena.user.id });
await say(jonas, dm.id, "Hey Lena, hast du die Fotos vom Wochenende noch?");
await say(lena, dm.id, "Ja, ich schicke sie dir heute Abend.");

// Lena read everything in allgemein except nothing in spiele and musik, so
// the sidebar shows unread state there.
await lena.call("PUT", `/channels/${g}/read`, {});

// Roles (v0.10), as in roles-editor.png: Moderation for Jonas, Stammgäste
// for Jonas, Mira and Tom; spiele is only for them, so Ada does not see it.
const moderators = await lena.call("POST", `/servers/${home.id}/roles`, {
  name: "Moderation",
  color: "blue",
  permissions: ["moderate_members", "manage_messages", "mention_everyone"],
});
const regulars = await lena.call("POST", `/servers/${home.id}/roles`, {
  name: "Stammgäste",
  color: "green",
});
await lena.call("PUT", `/servers/${home.id}/members/${jonas.user.id}/roles`, {
  role_ids: [moderators.id, regulars.id],
});
for (const p of [mira, tom])
  await lena.call("PUT", `/servers/${home.id}/members/${p.user.id}/roles`, {
    role_ids: [regulars.id],
  });
await lena.call("PATCH", `/channels/${games.id}`, {
  role_ids: [moderators.id, regulars.id],
});

// Betrieb (v0.10), for the operator scenes: two beta invitations by Lena,
// one bound to an address and one for anyone with the link, and an open
// report by Tom on Spieleabend. Only when the stack names Lena an operator.
let operator = true;
try {
  await lena.call("GET", "/operator");
} catch (error) {
  if (!String(error).includes(" 404 ")) throw error;
  operator = false;
  console.warn(
    "warning: lena is no operator (API_OPERATOR_EMAILS=lena@example.test); " +
      "seeded no invitations and no report",
  );
}
if (operator) {
  await lena.call("POST", "/operator/invitations", {
    email: "noah@example.test",
    note: "Noahs Spielegruppe",
    expires_in_days: 7,
  });
  await lena.call("POST", "/operator/invitations", {
    note: "Für den Stammtisch",
    expires_in_days: 30,
  });
  await tom.call("POST", `/servers/${evening.id}/reports`, {
    reason: "spam",
    details: "Im Kanal allgemein werden ständig Links zu einem Gewinnspiel gepostet.",
  });
}

// Entdecken (v0.10), as in directory.png: Ada's, Mira's and Tom's servers
// that Lena is not in, Jonas' Spieleabend that she is in ("Öffnen"), and her
// Wohnzimmer on request, with Kai's request waiting. Kai joins no server, so
// the member lists and "Neue Nachricht" stay as they were.
const listed = (client, server, description, language, joinMode) =>
  client.call("PUT", `/servers/${server}/listing`, {
    listed: true, description, language, join_mode: joinMode,
  });
for (const [owner, name, description, language, joinMode] of [
  [ada, "Filmclub", "Jeden Freitag ein Film im Kino-Kanal, danach wird gestritten.", "de", "open"],
  [mira, "Rust Study Hall", "Pair programming and code review in voice, beginners welcome.", "en", "open"],
  [tom, "Modellbau-Keller", "Bastelstreams und Fragen zu Farben, Kleber und Werkzeug.", "de", "request"],
]) {
  const server = await owner.call("POST", "/servers", { name });
  await listed(owner, server.id, description, language, joinMode);
}
await listed(jonas, evening.id, "Koop-Spiele und Brettspiel-Simulatoren, meist abends.", "de", "request");
await listed(lena, home.id, "Gemütliche Abende mit Freunden.", "de", "request");
await kai.call("POST", `/directory/listings/${home.id}/request`, {
  message: "Hallo, ich würde gern mitmachen.",
});

const fixture = {
  base: BASE,
  password: PASSWORD,
  users: Object.fromEntries(
    Object.entries(people).map(([k, p]) => [k, { id: p.user.id, email: `${k}@example.test`, name: p.name }]),
  ),
  servers: {
    home: { id: home.id, channels: { allgemein: g, spiele: games.id, musik: music.id, Sofa: sofa.id, Kino: cinema.id } },
    evening: { id: evening.id },
  },
  dm: dm.id,
};
await writeFile(OUT, JSON.stringify(fixture, null, 2) + "\n");
console.log(`seeded; fixture in ${OUT}`);
