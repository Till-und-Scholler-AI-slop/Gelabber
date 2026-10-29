// Chat history paging in a real engine (#201), without a server, an account
// or the network: headless Chromium runs the built app from a made-up
// origin; its files, /api and the gateway are answered from here.
// It checks what the unit tests cannot see: the reader scrolls up until the
// older page is asked for and stops; that page arrives about 400 ms later,
// well after the last scroll event, with rows much taller than the list's
// estimate. The row at the top of the list must stay where it was, while
// the new rows are laid out and measured above it. The same must hold when
// an edit arrives while that page is on its way, and for a reader who sits
// scrolled up while rows below are deleted and rows above grow (#215).
// Afterwards the reader must still be able to scroll away. A picture's
// lightbox closes when its message is deleted. A picture's box has the
// picture's own shape, at most its own size, and the list stays at its end
// as it shrinks to it. With unread messages the
// list opens at the first of them, after fetching the page it is on, and
// marks nothing read until asked to. A file says that its download started
// and repeated clicks save it once (#391). A toast above the composer leaves
// the newest message and its actions uncovered (#354).
// Run in web/: node scripts/smoke-chat-scroll.mjs
// GELABBER_CHAT_BROWSER_EXECUTABLE names a Chromium other than Playwright's.
/* global process, console, window, document, getComputedStyle, URL, requestAnimationFrame, setTimeout */
import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32, deflateSync } from "node:zlib";
import { chromium } from "playwright";
import { build } from "vite";

const web = fileURLToPath(new URL("..", import.meta.url));
// Secure, as the session's Web Locks need it, and without a server behind it.
const ORIGIN = "https://chat.test";
/** How long the older page is held back after it was asked for. */
const HOLD_MS = 400;
/** A 1×1 PNG for every chat picture. */
const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64",
);

/** A grey PNG of `width` × `height`: a screenshot of that shape. */
function png(width, height) {
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, crc]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // 8-bit grey
  // Every row: filter "none", then its pixels.
  const row = Buffer.alloc(width + 1, 0x80);
  row[0] = 0;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(Buffer.concat(Array(height).fill(row)))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
/** Pictures of their own size, by attachment id; any other is PIXEL. */
const pictures = new Map([
  // The wide screenshot of a single chat line, larger than the box.
  [id(5100), png(972, 114)],
  // Smaller than the box: shown at its own size.
  [id(5101), png(120, 40)],
]);
const ME = id(6);
const JONAS = id(7);
const SERVER = id(101);
const TEXT = id(201);
const AT = "2026-01-01T00:00:00Z";
const permissions = ["send_messages", "send_files"];
const server = {
  id: SERVER,
  name: "Wohnzimmer",
  owner_id: JONAS,
  created_at: AT,
  role: "member",
  permissions,
  member_permissions: permissions,
};
const person = (userId, name) => ({ id: userId, name, avatar_url: null });
const detail = {
  ...server,
  categories: [],
  channels: [
    {
      id: TEXT,
      server_id: SERVER,
      category_id: null,
      name: "allgemein",
      kind: "text",
      created_at: AT,
    },
  ],
  members: [
    { ...person(ME, "Lena"), user_id: ME, joined_at: AT, role: "member" },
    { ...person(JONAS, "Jonas"), user_id: JONAS, joined_at: AT, role: "owner" },
  ],
};
const WORDS =
  "Das ist ein ganz gewöhnlicher Satz in einer Unterhaltung, der eine Zeile füllt und dann weitergeht. ";
/** Message `n`, oldest first. Older ones are far taller than estimated. */
const message = (n) => ({
  id: id(1000 + n),
  revision: 1,
  created_order: n + 1,
  channel_id: TEXT,
  // Pairs by one author: some rows are continuations, estimated smaller.
  author: Math.floor(n / 2) % 2 ? person(ME, "Lena") : person(JONAS, "Jonas"),
  content: `Nachricht ${n}. ${WORDS.repeat(n < 50 ? 4 + (n % 5) * 3 : 1 + (n % 3))}`,
  created_at: new Date(Date.UTC(2026, 0, 1, 12, 0, n * 20)).toISOString(),
  edited_at: null,
  attachments: [],
  reactions: [],
});
const all = Array.from({ length: 100 }, (_, n) => message(n));
const newest = all.slice(50);
const older = all.slice(0, 50);

function answer(path, method, unread) {
  if (path === "/api/auth/session")
    return {
      user: { ...person(ME, "Lena"), email: "lena@example.test" },
      csrf_token: "a".repeat(64),
    };
  if (path === "/api/servers") return [server];
  if (path === `/api/servers/${SERVER}`) return detail;
  if (path === "/api/messages/unread") return unread ? [unread] : [];
  if (path === "/api/dms" || path === "/api/notification-settings") return [];
  if (path === "/api/me/themes")
    return { version: 1, revision: 1, active: "dark", customThemes: [] };
  if (path.endsWith("/read") || method !== "GET")
    return {
      channel_id: TEXT,
      server_id: SERVER,
      read_message_id: newest.at(-1).id,
      read_at: AT,
      unread_count: 0,
    };
  return {};
}

const frames = (page, count = 4) =>
  page.evaluate(async (left) => {
    while (left-- > 0) await new Promise(requestAnimationFrame);
  }, count);

/** The first row whose bottom is inside the list, and its top in it. */
const firstVisible = (page) =>
  page.evaluate(() => {
    const log = document.querySelector('[role="log"]');
    const top = log.getBoundingClientRect().top;
    const rows = [...log.querySelectorAll("[data-index]")]
      .map((row) => ({ row, rect: row.getBoundingClientRect() }))
      .filter(({ rect }) => rect.bottom > top + 1)
      .sort((a, b) => a.rect.top - b.rect.top);
    const first = rows[0];
    const text =
      first?.row.querySelector(".lr-message-text")?.textContent ?? "";
    return {
      name: text.split(".")[0],
      top: first ? first.rect.top - top : null,
      scrollTop: log.scrollTop,
    };
  });

/** Where the row `name` is now, relative to the list's top. */
const rowTop = (page, name) =>
  page.evaluate((wanted) => {
    const log = document.querySelector('[role="log"]');
    const row = [...log.querySelectorAll("[data-index]")].find((candidate) =>
      candidate
        .querySelector(".lr-message-text")
        ?.textContent?.startsWith(`${wanted}.`),
    );
    return row
      ? row.getBoundingClientRect().top - log.getBoundingClientRect().top
      : null;
  }, name);

const out = await mkdtemp(join(tmpdir(), "gelabber-chat-scroll-"));
let browser;
const problems = [];
let files;

/** The rows of `name` that are visible now: the first whose bottom is in. */
const visibleNames = (page) =>
  page.evaluate(() => {
    const log = document.querySelector('[role="log"]');
    const box = log.getBoundingClientRect();
    return [...log.querySelectorAll("[data-index]")]
      .map((row) => ({ row, rect: row.getBoundingClientRect() }))
      .filter(
        ({ rect }) => rect.bottom > box.top + 1 && rect.top < box.bottom - 1,
      )
      .sort((a, b) => a.rect.top - b.rect.top)
      .map(
        ({ row }) =>
          (row.querySelector(".lr-message-text")?.textContent ?? "").split(
            ".",
          )[0],
      );
  });

/**
 * A fresh page on the channel. The older page waits until `releaseOlder()`;
 * `send()` pushes a chat event for the channel through the gateway.
 */
async function open({ unread, shown = "Nachricht 99." } = {}) {
  const context = await browser.newContext({
    viewport: { width: 1100, height: 760 },
    locale: "de-DE",
    serviceWorkers: "block",
  });
  let olderAsked;
  let releaseOlder;
  /** Read positions the app wrote. */
  const marks = [];
  const asked = new Promise((resolve) => {
    olderAsked = resolve;
  });
  const held = new Promise((resolve) => {
    releaseOlder = resolve;
  });
  await context.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === `/api/channels/${TEXT}/messages`) {
      if (!url.searchParams.has("before"))
        return route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({ messages: newest, has_more: true }),
        });
      olderAsked(Date.now());
      await held;
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ messages: older, has_more: false }),
      });
    }
    if (url.pathname.endsWith("/read") && request.method() === "PUT")
      marks.push(request.postData());
    // A picture that loads, or the app shows its error card instead (#348).
    if (url.pathname.startsWith("/api/attachments/"))
      return route.fulfill(
        url.searchParams.has("download")
          ? {
              contentType: "application/pdf",
              headers: { "content-disposition": "attachment" },
              body: "%PDF-1.4\n",
            }
          : {
              contentType: "image/png",
              body: pictures.get(url.pathname.split("/")[3]) ?? PIXEL,
            },
      );
    if (url.pathname.startsWith("/api/"))
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(answer(url.pathname, request.method(), unread)),
      });
    return route.fulfill({
      path: join(out, files.has(url.pathname) ? url.pathname : "index.html"),
    });
  });
  let gateway;
  let subscribed;
  const channelSubscribed = new Promise((resolve) => {
    subscribed = resolve;
  });
  await context.routeWebSocket("**/ws", (socket) => {
    gateway = socket;
    socket.onMessage((raw) => {
      const frame = JSON.parse(raw);
      if (frame.op === "s") {
        socket.send(JSON.stringify({ op: "ok", s: frame.s, c: frame.c, n: 0 }));
        if (frame.c === TEXT) subscribed();
      }
    });
  });
  let sequence = 0;
  let revision = 1;
  const send = (event) =>
    gateway.send(
      JSON.stringify({
        op: "e",
        s: SERVER,
        c: TEXT,
        n: ++sequence,
        r: ++revision,
        ...event,
      }),
    );
  await context.addInitScript(() => {
    window.smokeErrors = [];
    window.addEventListener("error", (event) =>
      window.smokeErrors.push(event.message),
    );
  });
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", (error) => problems.push(error.message));
  await page.goto(`${ORIGIN}/s/${SERVER}/c/${TEXT}`);
  await page.getByText(shown).waitFor();
  await channelSubscribed;
  await frames(page, 8);
  const box = await page.locator('[role="log"]').boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const close = async () => {
    problems.push(...(await page.evaluate(() => window.smokeErrors)));
    await context.close();
  };
  return { page, asked, releaseOlder, send, close, marks };
}

/** An edit of message `n`: the same text (same length) or `content`. */
const edited = (n, content) => ({
  t: "e",
  d: {
    ...all[n],
    content: content ?? all[n].content,
    edited_at: "2026-01-01T13:00:00Z",
  },
});

/** Where `name` is over the next frames, relative to `top`. */
async function drift(page, name, top) {
  const seen = [];
  for (let pass = 0; pass < 12; pass++) {
    await frames(page, 4);
    seen.push(await rowTop(page, name));
  }
  return seen.map((at) => (at === null ? "off screen" : Math.round(at - top)));
}

const still = (moves) =>
  moves.every((delta) => typeof delta === "number" && Math.abs(delta) <= 2);

/**
 * The hold must end with the reader's scroll: a wheel up moves the row
 * down, and a later edit does not pull it back.
 */
async function scrollsAway({ page, send }, name, label) {
  const from = await rowTop(page, name);
  for (let step = 0; step < 3; step++) {
    await page.mouse.wheel(0, -120);
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  await frames(page, 8);
  const moved = await rowTop(page, name);
  send(edited(97));
  await frames(page, 12);
  const after = await rowTop(page, name);
  assert.ok(
    moved !== null && moved - from > 100,
    `${label}: the reader could not scroll away ("${name}" ${Math.round(from)} -> ${moved === null ? "off screen" : Math.round(moved)} px)`,
  );
  assert.ok(
    after !== null && Math.abs(after - moved) <= 2,
    `${label}: an edit pulled the list back after the reader scrolled ("${name}" ${Math.round(moved)} -> ${after === null ? "off screen" : Math.round(after)} px)`,
  );
  console.log(
    `PASS: ${label}: the reader scrolls away afterwards (+${Math.round(moved - from)} px) and an edit leaves it there`,
  );
}

/**
 * Scroll up in `wheel` px steps until the older page is asked for, stop,
 * then let it arrive `hold` ms later; `whileWaiting` runs before it does.
 */
async function olderPage(
  label,
  { wheel = 240, hold = HOLD_MS } = {},
  whileWaiting,
) {
  const opened = await open();
  const { page, asked, releaseOlder } = opened;
  let askedAt = null;
  asked.then((at) => {
    askedAt = at;
  });
  for (let step = 0; step < 200 && askedAt === null; step++) {
    await page.mouse.wheel(0, -wheel);
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  assert.ok(
    askedAt !== null,
    `${label}: scrolling up never asked for older history`,
  );

  // Stopped: let the last scroll settle, then note the top row.
  await frames(page, 6);
  const before = await firstVisible(page);
  assert.ok(before.top !== null, `${label}: no row at the top of the list`);
  await whileWaiting?.(opened);
  const wait = askedAt + hold - Date.now();
  if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
  releaseOlder();

  // Prepended: each row's index is now its message number.
  await page.waitForFunction(() =>
    [...document.querySelectorAll('[role="log"] [data-index]')].some((row) => {
      const text = row.querySelector(".lr-message-text")?.textContent ?? "";
      return text.startsWith(`Nachricht ${row.dataset.index}.`);
    }),
  );
  // Layout, measurement of the new rows above, and their corrections.
  const moves = await drift(page, before.name, before.top);
  assert.ok(
    still(moves),
    `${label}: "${before.name}" was ${Math.round(before.top)}px from the list's top and moved by ${moves.join(", ")} px while the older page was laid out`,
  );
  console.log(
    `PASS: ${label}: "${before.name}" stays at ${Math.round(before.top)}px while the older page (held ${hold} ms) is laid out and measured`,
  );
  await scrollsAway(opened, before.name, label);
  await opened.close();
}

try {
  await build({
    root: web,
    logLevel: "error",
    build: { outDir: out, emptyOutDir: true },
  });
  files = new Set(
    (await readdir(out, { recursive: true })).map((name) => `/${name}`),
  );
  browser = await chromium.launch({
    executablePath: process.env.GELABBER_CHAT_BROWSER_EXECUTABLE || undefined,
    headless: true,
    env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: "disabled:" },
  });

  {
    // Message 30 was the last one read: it is on the older page, which the
    // list fetches before it opens at message 31.
    const label = "unread";
    const opened = await open({
      unread: {
        channel_id: TEXT,
        server_id: SERVER,
        read_message_id: all[30].id,
        read_order: 31,
        read_at: AT,
        unread_count: 69,
      },
    });
    const { page, asked, releaseOlder, marks } = opened;
    await asked;
    releaseOlder();
    const divider = page.getByRole("separator", { name: "Neue Nachrichten" });
    await divider.waitFor();
    await frames(page, 8);
    const at = async () =>
      page.evaluate(() => {
        const log = document
          .querySelector('[role="log"]')
          .getBoundingClientRect();
        const line = document
          .querySelector(".lr-new-divider")
          .getBoundingClientRect();
        return { top: line.top - log.top, height: log.height };
      });
    const placed = await at();
    assert.ok(
      Math.abs(placed.top - placed.height / 3) <= 24,
      `${label}: "Neu" is ${Math.round(placed.top)}px from the top of a ${Math.round(placed.height)}px list, not a third down`,
    );
    const moves = await drift(
      page,
      "Nachricht 31",
      (await rowTop(page, "Nachricht 31")) ?? 0,
    );
    assert.ok(
      still(moves),
      `${label}: the first unread row moved by ${moves.join(", ")} px while rows were measured`,
    );
    await page.getByText("69 neue Nachrichten").waitFor();
    await new Promise((resolve) => setTimeout(resolve, 900));
    assert.deepEqual(
      marks,
      [],
      `${label}: marked read before the reader got there`,
    );
    console.log(
      `PASS: ${label}: opens with "Neu" ${Math.round(placed.top)}px down a ${Math.round(placed.height)}px list, holds it, and marks nothing read`,
    );
    // Reading down to the newest message marks it read.
    for (let step = 0; step < 80 && marks.length === 0; step++) {
      await page.mouse.wheel(0, 600);
      await new Promise((resolve) => setTimeout(resolve, 60));
    }
    await new Promise((resolve) => setTimeout(resolve, 900));
    assert.ok(
      marks.some((body) => body?.includes(all[99].id)),
      `${label}: reading to the end marked nothing read (${marks.join(", ")})`,
    );
    console.log(`PASS: ${label}: reading to the end marks it read`);
    await opened.close();

    const again = await open({
      unread: {
        channel_id: TEXT,
        server_id: SERVER,
        read_message_id: all[60].id,
        read_order: 61,
        read_at: AT,
        unread_count: 39,
      },
      shown: "Nachricht 61.",
    });
    await again.page.getByText("39 neue Nachrichten").waitFor();
    await again.page
      .getByRole("button", { name: /Als gelesen markieren/ })
      .click();
    await again.page
      .getByText("39 neue Nachrichten")
      .waitFor({ state: "detached" });
    assert.equal(
      again.marks.length,
      1,
      `${label}: "Als gelesen markieren" wrote no read position`,
    );
    console.log(
      `PASS: ${label}: "Als gelesen markieren" marks it read and the bar goes`,
    );
    await again.close();

    // The last read message was deleted since: its order still finds the
    // first unread one, on the older page.
    const deleted = await open({
      unread: {
        channel_id: TEXT,
        server_id: SERVER,
        read_message_id: id(999),
        read_order: 31,
        read_at: AT,
        unread_count: 69,
      },
    });
    await deleted.asked;
    deleted.releaseOlder();
    await deleted.page
      .getByRole("separator", { name: "Neue Nachrichten" })
      .waitFor();
    const after = await deleted.page.evaluate(
      () =>
        document
          .querySelector(".lr-new-divider")
          .closest("[data-index]")
          .querySelector(".lr-message-text")?.textContent,
    );
    assert.ok(
      after?.startsWith("Nachricht 31."),
      `${label}: with the read message deleted, "Neu" sits above ${after?.slice(0, 14)}`,
    );
    console.log(
      `PASS: ${label}: a deleted read message still opens at the first unread one`,
    );
    await deleted.close();
  }

  await olderPage("older page");
  // An edit commit while the page is on its way must not move the anchor
  // to a row rendered above the viewport. Longer steps leave the reader
  // mid-list when the page is asked for: before #215's fix, 38 px.
  await olderPage(
    "older page after an edit",
    { wheel: 500, hold: 600 },
    async ({ page, send }) => {
      send(edited(98));
      await frames(page, 6);
    },
  );

  {
    // A reader scrolled up a bit, not far enough to page, and idle.
    const label = "idle reader";
    const opened = await open();
    const { page, send } = opened;
    for (let step = 0; step < 5; step++) {
      await page.mouse.wheel(0, -240);
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
    await frames(page, 8);
    const before = await firstVisible(page);
    const top = Number(before.name.split(" ")[1]);
    const shown = await visibleNames(page);
    const below = Number(shown.at(-1).split(" ")[1]) + 2;
    assert.ok(
      top - 3 >= 50 && below <= 99,
      `${label}: unexpected rows in view (${shown.join(", ")})`,
    );
    // Same-length edit, a delete below the viewport, then a row three
    // above the viewport grows.
    send(edited(99));
    await frames(page, 6);
    send({ t: "d", i: all[below].id });
    await frames(page, 6);
    send(edited(top - 3, `${all[top - 3].content}${WORDS.repeat(12)}`));
    const moves = await drift(page, before.name, before.top);
    assert.ok(
      still(moves),
      `${label}: "${before.name}" was ${Math.round(before.top)}px from the list's top and moved by ${moves.join(", ")} px after a delete below and an edit above`,
    );
    console.log(
      `PASS: ${label}: "${before.name}" stays at ${Math.round(before.top)}px through a same-length edit, a delete below and a row above growing`,
    );
    await scrollsAway(opened, before.name, label);
    await opened.close();
  }

  {
    // The picture of a deleted message closes with it, and focus stays in
    // the list although the opening row is gone.
    const label = "lightbox";
    const opened = await open();
    const { page, send } = opened;
    const picture = {
      ...message(100),
      content: "Nachricht 100. Ein Bild.",
      attachments: [
        {
          id: id(5000),
          filename: "bild.png",
          content_type: "image/png",
          size: 1,
        },
      ],
    };
    send({ t: "c", d: picture });
    const link = page.getByRole("link", { name: "bild.png vergrößern" });
    await link.click();
    const dialog = page.getByRole("dialog", { name: "bild.png" });
    await dialog.waitFor();
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });
    assert.ok(
      await link.evaluate((element) => element === document.activeElement),
      `${label}: focus did not return to the picture after Esc`,
    );
    await link.click();
    await dialog.waitFor();
    send({ t: "d", i: picture.id });
    await dialog.waitFor({ state: "detached" });
    const focus = await page.evaluate(() => {
      const active = document.activeElement;
      return {
        inList: !!active?.closest('[role="log"]'),
        tag: active?.tagName,
      };
    });
    assert.ok(
      focus.inList,
      `${label}: focus went to ${focus.tag}, not the list, after the message was deleted`,
    );
    console.log(
      `PASS: ${label}: Esc returns focus to the picture; deleting its message closes it and focus stays in the list`,
    );
    await opened.close();
  }

  {
    // A picture's box is the picture: a small, wide screenshot got the
    // 240 px box of a picture still loading, the rest of it empty.
    const label = "picture size";
    const opened = await open();
    const { page, send } = opened;
    const shots = [
      [id(5100), "zeile.png", 101],
      [id(5101), "klein.png", 102],
    ];
    for (const [attachment, filename, n] of shots)
      send({
        t: "c",
        d: {
          ...message(n),
          content: `Nachricht ${n}. Ein Screenshot.`,
          attachments: [
            {
              id: attachment,
              filename,
              content_type: "image/png",
              size: pictures.get(attachment).length,
            },
          ],
        },
      });
    await page.waitForFunction(() => {
      const shown = document.querySelectorAll(".lr-attachment-image img");
      return (
        shown.length === 2 &&
        [...shown].every((img) => img.complete && img.naturalWidth > 1)
      );
    });
    await frames(page, 8);
    const placed = await page.evaluate(() => {
      const log = document.querySelector('[role="log"]');
      return {
        atEnd: log.scrollHeight - log.scrollTop - log.clientHeight <= 2,
        boxes: [...log.querySelectorAll(".lr-attachment-image")].map((box) => {
          const img = box.querySelector("img");
          const outer = box.getBoundingClientRect();
          const drawn = img.getBoundingClientRect();
          return {
            name: img.alt,
            box: [outer.width, outer.height],
            // Without its 1 px border.
            drawn: [drawn.width - 2, drawn.height - 2],
            natural: [img.naturalWidth, img.naturalHeight],
          };
        }),
      };
    });
    const wanted = { "zeile.png": [358, 42], "klein.png": [120, 40] };
    for (const { name, box, drawn, natural } of placed.boxes) {
      const size = (pair) => pair.map((v) => Math.round(v)).join("×");
      assert.equal(
        size(drawn),
        size(wanted[name]),
        `${label}: ${name} (${size(natural)}) is drawn at ${size(drawn)}`,
      );
      assert.ok(
        Math.abs(box[0] - drawn[0] - 2) < 1 &&
          Math.abs(box[1] - drawn[1] - 2) < 1,
        `${label}: ${name} is drawn at ${size(drawn)} in a box of ${size(box)}`,
      );
    }
    assert.equal(placed.boxes.length, 2);
    assert.ok(
      placed.atEnd,
      `${label}: the list left its end as the pictures loaded`,
    );
    console.log(
      `PASS: ${label}: ${placed.boxes.map(({ name, natural, box }) => `${name} ${natural.join("×")} in a box of ${box.map(Math.round).join("×")}`).join(", ")}; the list stays at its end`,
    );
    await opened.close();
  }

  {
    // A file says that its download started, and the clicks of someone who
    // did not believe it start no further copies (#391).
    const label = "download";
    const opened = await open();
    const { page, send } = opened;
    const downloads = [];
    page.on("download", (download) =>
      downloads.push(download.suggestedFilename()),
    );
    send({
      t: "c",
      d: {
        ...message(100),
        content: "Nachricht 100. Ein Dokument.",
        attachments: [
          {
            id: id(5001),
            filename: "vertrag.pdf",
            content_type: "application/pdf",
            size: 48 * 1024,
          },
        ],
      },
    });
    const row = page.locator(".lr-attachment-file", { hasText: "vertrag.pdf" });
    const started = row.getByRole("status");
    await row.click();
    await started.filter({ hasText: "Download gestartet" }).waitFor();
    // Plain clicks: Playwright's own would wait for the link to be enabled.
    const box = await row.boundingBox();
    for (let n = 0; n < 4; n++)
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await new Promise((resolve) => setTimeout(resolve, 500));
    assert.deepEqual(
      downloads,
      ["vertrag.pdf"],
      `${label}: five clicks started ${downloads.length} downloads`,
    );
    await started.waitFor({ state: "detached", timeout: 8000 });
    await row.getByText("48 KB").waitFor();
    await row.click();
    await started.waitFor();
    await new Promise((resolve) => setTimeout(resolve, 500));
    assert.equal(
      downloads.length,
      2,
      `${label}: a click after the notice did not download again`,
    );
    console.log(
      `PASS: ${label}: five clicks save one copy and say so; once the notice is gone, a click saves the file again`,
    );
    await opened.close();
  }

  {
    // A toast above the composer must not cover the newest message or its
    // actions (#354): the list makes room and stays at its end.
    const label = "toast";
    const opened = await open();
    const { page, send } = opened;
    // One short line that continues the last author's: its actions sit
    // lowest, where the toast goes.
    send({
      t: "c",
      d: {
        ...message(100),
        author: all[99].author,
        content: "Nachricht 100. Kurz.",
      },
    });
    const newest = page
      .locator('[role="log"] [data-index]')
      .filter({ hasText: "Nachricht 100." });
    await newest.waitFor();
    await frames(page, 8);
    await newest.hover();
    await newest.getByRole("button", { name: "Weitere Aktionen" }).click();
    await page.getByRole("menuitem", { name: "Text kopieren" }).click();
    const toast = page.locator(".ui-toast");
    await toast.waitFor();
    // The toast's rise and the room it takes.
    await new Promise((resolve) => setTimeout(resolve, 400));
    await newest.hover();
    await frames(page, 4);
    const placed = await page.evaluate(() => {
      const log = document.querySelector('[role="log"]');
      const rows = log.querySelectorAll("[data-index]");
      const row = rows[rows.length - 1];
      const actions = row.querySelector(".lr-message-actions");
      const tools = actions.getBoundingClientRect();
      const card = document.querySelector(".ui-toast").getBoundingClientRect();
      const hit = document.elementFromPoint(
        tools.left + tools.width / 2,
        tools.top + tools.height / 2,
      );
      return {
        hit: actions.contains(hit),
        rowBottom: Math.round(row.getBoundingClientRect().bottom),
        listBottom: Math.round(log.getBoundingClientRect().bottom),
        toastTop: Math.round(card.top),
        atEnd: log.scrollHeight - log.scrollTop - log.clientHeight <= 2,
      };
    });
    assert.ok(
      placed.hit,
      `${label}: the newest message's actions are covered while a toast shows`,
    );
    assert.ok(
      placed.listBottom <= placed.toastTop && placed.atEnd,
      `${label}: the list ends at ${placed.listBottom}px (at its end: ${placed.atEnd}), the toast starts at ${placed.toastTop}px`,
    );
    await toast.getByRole("button", { name: "Schließen" }).click();
    await toast.waitFor({ state: "detached" });
    await new Promise((resolve) => setTimeout(resolve, 400));
    const after = await page.evaluate(() => {
      const log = document.querySelector('[role="log"]');
      return {
        space: getComputedStyle(document.documentElement)
          .getPropertyValue("--lr-toast-space")
          .trim(),
        listBottom: Math.round(log.getBoundingClientRect().bottom),
        atEnd: log.scrollHeight - log.scrollTop - log.clientHeight <= 2,
      };
    });
    assert.ok(
      after.space === "0px" && after.atEnd,
      `${label}: after the toast the room is ${after.space} and the list is ${after.atEnd ? "" : "not "}at its end`,
    );
    console.log(
      `PASS: ${label}: the list gives ${after.listBottom - placed.listBottom}px to a toast, keeps the newest message's actions reachable, and takes the room back`,
    );
    await opened.close();
  }

  // Scrolling up through rows not measured yet makes Chromium report a
  // ResizeObserver loop; main without #201 shows the same. Said, not failed.
  const loop = "ResizeObserver loop completed with undelivered notifications.";
  const loops = problems.filter((problem) => problem === loop).length;
  assert.deepEqual(
    problems.filter((problem) => problem !== loop),
    [],
    "the page reported errors",
  );
  console.log(
    `PASS: no page errors${loops ? ` (besides ${loops} ResizeObserver loop notices while scrolling)` : ""}, ${browser.browserType().name()} ${browser.version()}`,
  );
} finally {
  await browser?.close();
  await rm(out, { recursive: true, force: true });
}
