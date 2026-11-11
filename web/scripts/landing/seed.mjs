// Seeds the server shown on the landing page's screenshot
// (docs/design/v0.10/landing-preview-en.png): Lena's "Living Room" with
// general, games, music and the voice rooms Lounge, Sofa and Cinema, the
// people Mira, Tom, Lena, Jonas and Ada, and the preview's conversation.
// `de` seeds the same shape with German names and text.
// Each language has accounts of its own (lena.en@…, lena.de@…), so Lena's
// server rail shows only that language's servers. Lena's account takes the
// language, so the server's first channels are named in it; the others stay
// German, as call.mjs presses the German buttons.
// Run in web/ against a local stack, as for the parity shots:
//   node scripts/landing/seed.mjs [en|de] [base-url]
// The base URL is the web origin (Vite proxies /api), default 127.0.0.1:5180.
// It writes scripts/landing/fixture-<lang>.json, in the shape of the parity
// fixture, for shot.mjs and call.mjs.
/* global process, console, fetch, URL */
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

const LANG = process.argv[2] ?? "en";
if (!["en", "de"].includes(LANG))
  throw new Error(`language en or de, not ${LANG}`);
const BASE = process.argv[3] ?? "http://127.0.0.1:5180";
const here = new URL(".", import.meta.url).pathname;
const OUT = join(here, `fixture-${LANG}.json`);
const PASSWORD = "landing-pass-123";

const TEXT = {
  en: {
    server: "Living Room",
    other: "Games night",
    chatting: "Chatting",
    games: "games",
    music: "music",
    cinema: "Cinema",
    chat: [
      ["jonas", "Hello everyone!"],
      ["jonas", "Who’s in tonight?"],
      ["mira", "I’m in. **Film or game?**", "plan"],
      ["tom", "Game! I’d be up for *board games* or something co-op."],
      ["lena", "Sounds good. @Jonas, are you bringing the snacks?"],
      ["jonas", "Sure, I’ll bring crisps and lemonade.", "reply"],
      [
        "ada",
        "I’ve already looked up the rules:\n> Each round, everyone draws two cards.\n> Whoever reaches ten points first wins.",
      ],
      [
        "tom",
        "I wrote a small script for the points:\n```\nfn points(cards: &[u8]) -> u32 {\n    cards.iter().map(|c| *c as u32).sum()\n}\n```",
      ],
      ["mira", "Perfect, then let’s meet at `20:00` in Sofa."],
      ["lena", "See you later!"],
    ],
    gamesChat: [
      ["tom", "Another round on Friday?"],
      ["ada", "Happy to, from 7 pm."],
    ],
    musicChat: [["jonas", "Listening to anything good right now?"]],
    dm: "Hey Lena, do you still have the photos from the weekend?",
  },
  de: {
    server: "Wohnzimmer",
    other: "Spieleabend",
    chatting: "Plaudern",
    games: "spiele",
    music: "musik",
    cinema: "Kino",
    chat: [
      ["jonas", "Hallo zusammen!"],
      ["jonas", "Wer ist heute Abend dabei?"],
      ["mira", "Ich bin dabei. **Film oder Spiel?**", "plan"],
      [
        "tom",
        "Spiel! Ich hätte Lust auf *Brettspiele* oder etwas Kooperatives.",
      ],
      ["lena", "Klingt gut. @Jonas, bringst du die Snacks mit?"],
      ["jonas", "Klar, ich bring Chips und Limo mit.", "reply"],
      [
        "ada",
        "Ich habe die Regeln schon mal rausgesucht:\n> Jede Runde zieht jede Person zwei Karten.\n> Wer zuerst zehn Punkte hat, gewinnt.",
      ],
      [
        "tom",
        "Für die Punkte habe ich ein kleines Skript:\n```\nfn punkte(karten: &[u8]) -> u32 {\n    karten.iter().map(|k| *k as u32).sum()\n}\n```",
      ],
      ["mira", "Perfekt, dann treffen wir uns um `20:00` im Sofa."],
      ["lena", "Bis später!"],
    ],
    gamesChat: [
      ["tom", "Neue Runde am Freitag?"],
      ["ada", "Gerne, ab 19 Uhr."],
    ],
    musicChat: [["jonas", "Hört ihr gerade was Gutes?"]],
    dm: "Hey Lena, hast du die Fotos vom Wochenende noch?",
  },
}[LANG];

const PEOPLE = [
  ["lena", "Lena"],
  ["jonas", "Jonas"],
  ["mira", "Mira"],
  ["tom", "Tom"],
  ["ada", "Ada"],
];
const email = (handle) => `${handle}.${LANG}@example.test`;

class Client {
  constructor(name) {
    this.name = name;
    this.cookies = new Map();
    this.csrf = null;
  }

  async call(method, path, body) {
    const headers = { Accept: "application/json" };
    if (this.cookies.size)
      headers.Cookie = [...this.cookies]
        .map(([k, v]) => `${k}=${v}`)
        .join("; ");
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
      throw new Error(
        `${this.name}: ${method} ${path} → ${res.status} ${text}`,
      );
    return data;
  }

  async refreshCsrf() {
    const session = await this.call("GET", "/auth/session");
    this.csrf = session.csrf_token;
    return session;
  }

  async signIn(handle, name, locale) {
    await this.refreshCsrf();
    try {
      await this.call("POST", "/auth/register", {
        email: email(handle),
        password: PASSWORD,
        name,
        locale,
      });
    } catch (error) {
      if (!String(error).includes(" 409 ")) throw error;
      await this.call("POST", "/auth/login", {
        email: email(handle),
        password: PASSWORD,
      });
    }
    const session = await this.refreshCsrf();
    this.user = session.user;
    return this;
  }
}

const people = {};
for (const [handle, name] of PEOPLE)
  people[handle] = await new Client(name).signIn(
    handle,
    name,
    handle === "lena" ? LANG : "de",
  );
const { lena, jonas, mira, tom, ada } = people;
// The language the server's first channels are named in.
await lena.call("PUT", "/me/locale", { locale: LANG });

const existing = await lena.call("GET", "/servers");
if (existing.some((s) => s.name === TEXT.server)) {
  console.log("already seeded; recreate the database to seed again");
  process.exit(0);
}

// Lena's server, everyone joins. Its first voice room becomes Lounge.
const home = await lena.call("POST", "/servers", { name: TEXT.server });
const detail = await lena.call("GET", `/servers/${home.id}`);
const general = detail.channels.find((c) => c.kind === "text");
const lounge = detail.channels.find((c) => c.kind === "voice");
await lena.call("PATCH", `/channels/${lounge.id}`, { name: "Lounge" });
const voiceCat = detail.categories.find((c) => c.id === lounge.category_id);
const textCat = await lena.call("POST", `/servers/${home.id}/categories`, {
  name: TEXT.chatting,
});
const channel = (name, kind, category) =>
  lena.call("POST", `/servers/${home.id}/channels`, {
    name,
    kind,
    category_id: category.id,
  });
const games = await channel(TEXT.games, "text", textCat);
const music = await channel(TEXT.music, "text", textCat);
const sofa = await channel("Sofa", "voice", voiceCat);
const cinema = await channel(TEXT.cinema, "voice", voiceCat);
const invite = await lena.call("POST", `/servers/${home.id}/invites`, {});
for (const p of [jonas, mira, tom, ada])
  await p.call("POST", `/invites/${invite.code}/join`);

// Jonas' server, so the rail shows a second one.
const other = await jonas.call("POST", "/servers", { name: TEXT.other });
const otherInvite = await jonas.call(
  "POST",
  `/servers/${other.id}/invites`,
  {},
);
await lena.call("POST", `/invites/${otherInvite.code}/join`);

const say = (who, where, content, extra = {}) =>
  people[who].call("POST", `/channels/${where}/messages`, {
    content,
    ...extra,
  });
// The preview's conversation; Jonas answers Mira's question ("plan").
let plan = null;
for (const [who, content, mark] of TEXT.chat) {
  const message = await say(
    who,
    general.id,
    content,
    mark === "reply" ? { reply_to: plan.id } : {},
  );
  if (mark === "plan") plan = message;
}
for (const [who, content] of TEXT.gamesChat) await say(who, games.id, content);
for (const [who, content] of TEXT.musicChat) await say(who, music.id, content);
const dm = await jonas.call("POST", "/dms", { user_id: lena.user.id });
await say("jonas", dm.id, TEXT.dm);

// Lena has read general; games, music and the DM stay unread.
await lena.call("PUT", `/channels/${general.id}/read`, {});

const fixture = {
  base: BASE,
  password: PASSWORD,
  locale: LANG,
  users: Object.fromEntries(
    Object.entries(people).map(([k, p]) => [
      k,
      { id: p.user.id, email: email(k), name: p.name },
    ]),
  ),
  servers: {
    home: {
      id: home.id,
      channels: {
        general: general.id,
        games: games.id,
        music: music.id,
        Lounge: lounge.id,
        Sofa: sofa.id,
        Cinema: cinema.id,
      },
    },
    other: { id: other.id },
  },
  dm: dm.id,
};
await writeFile(OUT, JSON.stringify(fixture, null, 2) + "\n");
console.log(`seeded ${LANG}; fixture in ${OUT}`);
