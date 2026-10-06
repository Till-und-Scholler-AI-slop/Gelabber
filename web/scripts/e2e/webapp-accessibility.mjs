/* global process, document, window, console, getComputedStyle */
// Local native-browser keyboard and viewport acceptance. Fake capture is used
// for the real local call; this does not prove physical iOS/Omarchy behavior.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { chromium, firefox } from "playwright";
import { api, until, safeTarget } from "./harness.mjs";
import { browserLaunchOptions } from "./browser-options.mjs";

const base = safeTarget(
  process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
);
const output =
  process.env.GELABBER_ACCESSIBILITY_REPORT ??
  "/tmp/gelabber-webapp-accessibility";
await mkdir(output, { recursive: true });
const engine = process.env.GELABBER_A11Y_BROWSER ?? "chromium";
assert.ok(["chromium", "firefox"].includes(engine));
const browser = await { chromium, firefox }[engine].launch(
  browserLaunchOptions(engine),
);
const version = browser.version();
const suffix = randomBytes(6).toString("hex");
const password = randomBytes(20).toString("base64url");
const contexts = [],
  checks = [],
  screenshots = [],
  failures = [],
  errors = [];
let owner, member, server;

async function actor(label) {
  const context = await browser.newContext({ hasTouch: engine === "chromium" });
  contexts.push(context);
  const page = await context.newPage();
  page.setDefaultTimeout(12_000);
  page.on("pageerror", (error) => errors.push(error.message));
  const who = {
    context,
    page,
    email: `a11y-${label}-${suffix}@example.test`,
    password,
  };
  await page.goto(`${base}/register`);
  await page.getByLabel("Name", { exact: true }).fill(`A11y ${label}`);
  await page.getByLabel("E-Mail-Adresse").fill(who.email);
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Registrieren", exact: true }).click();
  await page.waitForURL((url) => !/(login|register)$/.test(url.pathname));
  return who;
}

async function capture(name) {
  const page = member.page;
  await page.screenshot({ path: `${output}/${name}.png` });
  const state = await page.evaluate(() => {
    const active = document.activeElement;
    const rect = (element) => {
      const r = element.getBoundingClientRect();
      return {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
        bottom: r.bottom,
      };
    };
    return {
      text: document.body.innerText,
      viewport: { width: window.innerWidth, height: window.innerHeight },
      scrollWidth: document.documentElement.scrollWidth,
      focus: active
        ? {
            tag: active.tagName,
            label: active.getAttribute("aria-label") ?? active.textContent,
            ...rect(active),
          }
        : null,
      dialogs: [...document.querySelectorAll("dialog[open]")].map((d) => ({
        label: d.getAttribute("aria-labelledby"),
        ...rect(d),
      })),
      dock: document.querySelector(".voice-session-dock")
        ? rect(document.querySelector(".voice-session-dock"))
        : null,
      composer: document.querySelector("form textarea")
        ? rect(document.querySelector("form textarea"))
        : null,
      pane: document.querySelector(".lr-message-pane")
        ? {
            ...rect(document.querySelector(".lr-message-pane")),
            height: document.querySelector(".lr-message-pane").clientHeight,
            scrollHeight:
              document.querySelector(".lr-message-pane").scrollHeight,
            top: document.querySelector(".lr-message-pane").scrollTop,
            overflow: getComputedStyle(
              document.querySelector(".lr-message-pane"),
            ).overflowY,
            dockSpace: getComputedStyle(
              document.documentElement,
            ).getPropertyValue("--lr-media-space"),
          }
        : null,
    };
  });
  await writeFile(`${output}/${name}.json`, JSON.stringify(state, null, 2));
  screenshots.push(name);
  assert.ok(
    state.scrollWidth <= state.viewport.width,
    `${name}: horizontal document overflow`,
  );
  return state;
}
const focused = (locator) =>
  locator.evaluate((element) => document.activeElement === element);
async function tabTo(locator) {
  for (let i = 0; i < 100; i++) {
    if (await focused(locator)) return;
    await member.page.keyboard.press("Tab");
  }
  throw new Error(
    `Keyboard cannot reach ${(await locator.getAttribute("aria-label")) ?? (await locator.textContent())}`,
  );
}
async function check(name, run) {
  if (
    process.env.GELABBER_A11Y_ONLY &&
    !name.includes(process.env.GELABBER_A11Y_ONLY)
  )
    return;
  try {
    await run();
    checks.push(name);
    console.log(`PASS ${name}`);
  } catch (error) {
    failures.push({ name, message: error.message });
    await capture(`failure-${failures.length}`).catch(() => {});
    console.log(`FAIL ${name}: ${error.message}`);
  } finally {
    // Failure must not leave a native modal blocking the next phase.
    if (member && (await member.page.locator("dialog:modal").count()))
      await member.page.keyboard.press("Escape");
  }
}
const sizes = [
  [320, 740],
  [390, 844],
  [390, 400],
  [844, 390],
  [1280, 720],
];
async function matrix(prefix, ready) {
  for (const [width, height] of sizes) {
    await member.page.setViewportSize({ width, height });
    await ready();
    await capture(`${prefix}-${width}x${height}`);
  }
}

try {
  owner = await actor("owner");
  member = await actor("member");
  const created = await api(owner, "/servers", "POST", {
    name: `Accessibility ${suffix}`,
  });
  assert.equal(created.status, 201);
  server = created.body.id;
  const textId = created.body.channels.find(
    (channel) => channel.kind === "text",
  ).id;
  const voice = await api(owner, `/servers/${server}/channels`, "POST", {
    name: "keyboard-call",
    kind: "voice",
  });
  assert.equal(voice.status, 201);
  const invite = (await api(owner, `/servers/${server}/invites`, "POST", {}))
    .body.code;
  assert.equal(
    (await api(member, `/invites/${invite}/join`, "POST")).status,
    200,
  );
  const needle = `keyboardneedle ${suffix}`;
  const message = await api(owner, `/channels/${textId}/messages`, "POST", {
    content: needle,
  });
  assert.equal(message.status, 201);
  const chatPath = `/s/${server}/c/${textId}`,
    voicePath = `/s/${server}/c/${voice.body.id}`;
  const page = member.page;
  await page.goto(`${base}${chatPath}`);
  const log = page.getByRole("log", { name: "Nachrichten" });
  await log.getByText(needle, { exact: true }).waitFor();
  await check(
    "chat fits 320px, phone, short-height, rotation and desktop",
    () => matrix("01-chat", () => log.waitFor()),
  );

  await check(
    "navigation and members trap Tab and restore focus on Escape",
    async () => {
      await page.setViewportSize({ width: 320, height: 740 });
      const opener = page.getByRole("button", {
        name: "Navigation öffnen",
        exact: true,
      });
      await opener.focus();
      await page.keyboard.press("Enter");
      const navigation = page.getByRole("dialog", {
        name: "Navigation",
        exact: true,
      });
      await navigation.locator(`a[href="${chatPath}"]`).waitFor();
      assert.ok(
        await navigation.evaluate((el) => el.contains(document.activeElement)),
      );
      for (let i = 0; i < 25; i++) {
        await page.keyboard.press("Tab");
        assert.ok(
          await navigation.evaluate(
            (el) =>
              el.contains(document.activeElement) ||
              document.activeElement === document.body,
          ),
          `Tab reached the background app from Navigation at ${i}`,
        );
      }
      await capture("02-navigation-tab");
      await page.keyboard.press("Escape");
      await navigation.waitFor({ state: "hidden" });
      assert.ok(await focused(opener));
      await capture("03-navigation-escape");
      const members = page.getByRole("button", { name: /Mitglieder/ });
      await tabTo(members);
      await page.keyboard.press("Enter");
      const panel = page.getByRole("dialog", {
        name: "Mitglieder",
        exact: true,
      });
      await panel.waitFor();
      for (let i = 0; i < 10; i++) {
        await page.keyboard.press("Tab");
        assert.ok(
          await panel.evaluate(
            (el) =>
              el.contains(document.activeElement) ||
              document.activeElement === document.body,
          ),
        );
      }
      await capture("04-members-tab");
      await page.keyboard.press("Escape");
      assert.ok(await focused(members));
    },
  );

  await check(
    "reaction search, native Tab/Enter selection and Escape return focus",
    async () => {
      await page.setViewportSize({ width: 320, height: 740 });
      const row = log.locator(`[data-index]`).filter({ hasText: needle });
      const add = row.getByRole("button", {
        name: "Reaktion hinzufügen",
        exact: true,
      });
      await add.focus();
      await page.keyboard.press("Enter");
      const picker = page.getByRole("dialog", {
        name: "Reaktion auswählen",
        exact: true,
      });
      const search = picker.getByRole("searchbox");
      await until(() => focused(search), Boolean, "reaction-search-focus");
      await page.keyboard.press("Escape");
      await picker.waitFor({ state: "hidden" });
      assert.ok(await focused(add), "first picker Escape lost opener focus");
      // The cached picker mounts its autofocus child in the same React commit.
      await page.keyboard.press("Enter");
      await until(
        () => focused(search),
        Boolean,
        "cached-reaction-search-focus",
      );
      await page.keyboard.press("Escape");
      await picker.waitFor({ state: "hidden" });
      await capture("05a-cached-picker-escape");
      assert.ok(await focused(add), "cached picker Escape lost opener focus");
      await page.keyboard.press("Enter");
      await until(
        () => focused(search),
        Boolean,
        "selection-reaction-search-focus",
      );
      await page.keyboard.type("rotes Herz");
      const heart = picker.getByRole("button", {
        name: "rotes Herz",
        exact: true,
      });
      await heart.waitFor();
      await tabTo(heart);
      await capture("05-reaction-keyboard");
      await page.keyboard.press("Enter");
      await picker.waitFor({ state: "hidden" });
      await until(
        () => add.getAttribute("aria-disabled"),
        (value) => value !== "true",
        "selected-reaction-settled",
      );
      assert.ok(await focused(add));
      const own = row.getByRole("button", {
        name: "❤️: 1 Reaktionen, du hast reagiert",
        exact: true,
      });
      await own.waitFor();
      await add.focus();
      await page.keyboard.press("Enter");
      await picker.waitFor();
      await page.keyboard.press("Escape");
      assert.ok(await focused(add));
      await capture("06-reaction-escape");
      await own.focus();
      await page.keyboard.press("Enter");
      await own.waitFor({ state: "detached" });
      await capture("06a-removed-chip-focus");
      assert.ok(
        await focused(add),
        "removing the last focused chip lost opener focus",
      );
    },
  );

  await check(
    "autofocus server dialog Escape restores its opener",
    async () => {
      await page.setViewportSize({ width: 1280, height: 720 });
      const opener = page.getByRole("button", {
        name: "Server erstellen",
        exact: true,
      });
      await opener.focus();
      await page.keyboard.press("Enter");
      const dialog = page.getByRole("dialog", {
        name: "Server erstellen",
        exact: true,
      });
      await until(
        () => focused(dialog.getByLabel("Name", { exact: true })),
        Boolean,
        "server-name-autofocus",
      );
      await capture("06a-server-autofocus");
      await page.keyboard.press("Escape");
      await dialog.waitFor({ state: "hidden" });
      await capture("06b-server-escape");
      assert.ok(
        await focused(opener),
        "server dialog Escape lost opener focus",
      );
    },
  );

  await check(
    "search opens context by keyboard and closing restores its trigger",
    async () => {
      const trigger = page.getByRole("button", {
        name: "Nachrichten suchen",
        exact: true,
      });
      await trigger.focus();
      await page.keyboard.press("Enter");
      const search = page.getByRole("searchbox");
      assert.ok(await focused(search));
      await page.keyboard.type(needle);
      await page.keyboard.press("Enter");
      const hit = page.getByRole("button", {
        name: `Zur Nachricht von A11y owner: ${needle}`,
        exact: true,
      });
      await hit.waitFor();
      await tabTo(hit);
      await capture("07-search-keyboard");
      await page.keyboard.press("Enter");
      const context = page.getByRole("region", {
        name: "Nachrichtenkontext",
        exact: true,
      });
      const target = context.locator(`[data-message-id="${message.body.id}"]`);
      await until(() => focused(target), Boolean, "message-target-focus");
      await capture("08-context-keyboard");
      const back = context.getByRole("button", {
        name: "Zurück zu den Treffern",
        exact: true,
      });
      await back.focus();
      await page.keyboard.press("Enter");
      assert.ok(await focused(hit));
      const close = page.getByRole("button", {
        name: "Zurück zum Chat",
        exact: true,
      });
      await close.focus();
      await page.keyboard.press("Enter");
      await capture("09-search-closed-focus");
      assert.ok(await focused(trigger), "closing search lost keyboard focus");
    },
  );

  await check(
    "settings areas are reachable with native Tab/Enter at short height",
    async () => {
      await page.goto(`${base}/settings`);
      await page
        .getByRole("heading", { name: "Einstellungen", exact: true })
        .waitFor();
      const nav = page.getByRole("navigation", {
        name: "Einstellungsbereiche",
        exact: true,
      });
      await matrix("10-settings", () =>
        page
          .getByRole("heading", { name: "Darstellung", exact: true })
          .waitFor(),
      );
      await page.setViewportSize({ width: 390, height: 400 });
      const appearance = nav.getByRole("button", {
        name: "Darstellung",
        exact: true,
      });
      await appearance.focus();
      for (const area of ["Themes", "Audio", "Video", "Benachrichtigungen"]) {
        const button = nav.getByRole("button", { name: area, exact: true });
        await tabTo(button);
        await page.keyboard.press("Enter");
        const heading = page.getByRole("heading", { name: area, exact: true });
        await heading.waitFor();
        await heading.scrollIntoViewIfNeeded();
        await capture(`11-settings-${area}`);
        assert.equal(await button.getAttribute("aria-current"), "page");
      }
      const firstControl = page
        .locator(
          "#settings-content input, #settings-content select, #settings-content button",
        )
        .first();
      await tabTo(firstControl);
      await capture("12-settings-control-focus");
      const box = await firstControl.boundingBox();
      assert.ok(
        box && box.y >= 0 && box.y + box.height <= 400,
        "focused settings control is obscured",
      );
    },
  );

  await check(
    "history PageDown/End works while picker and message actions preserve the anchor",
    async () => {
      await page.goto(`${base}${chatPath}`);
      await page.setViewportSize({ width: 1280, height: 720 });
      const own = (
        await api(member, `/channels/${textId}/messages`, "POST", {
          content: "Keyboard owned action target",
        })
      ).body;
      for (let i = 0; i < 25; i++) {
        assert.equal(
          (
            await api(owner, `/channels/${textId}/messages`, "POST", {
              content: `Keyboard history ${i}`,
            })
          ).status,
          201,
        );
      }
      await log.getByText("Keyboard history 24", { exact: true }).waitFor();
      const lastAdd = log
        .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
        .last();
      await lastAdd.focus();
      await page.keyboard.press("Home");
      await until(
        () => log.evaluate((el) => el.scrollTop),
        (top) => top < 30,
        "native-history-home",
      );
      const oldRow = log.locator("[data-index]").filter({ hasText: needle });
      const add = oldRow.getByRole("button", {
        name: "Reaktion hinzufügen",
        exact: true,
      });
      await add.focus();
      const before = await log.evaluate((el) => el.scrollTop);
      await page.keyboard.press("Enter");
      const picker = page.getByRole("dialog", {
        name: "Reaktion auswählen",
        exact: true,
      });
      const emoji = picker.getByRole("button").first();
      await emoji.waitFor();
      await tabTo(emoji);
      await page.keyboard.press("End");
      await capture("12a-picker-end-history");
      assert.equal(
        await log.evaluate((el) => el.scrollTop),
        before,
        "End inside picker jumped canonical history",
      );
      await page.keyboard.press("PageDown");
      assert.equal(
        await log.evaluate((el) => el.scrollTop),
        before,
        "PageDown inside picker jumped canonical history",
      );
      await page.keyboard.press("Escape");
      const ownedRow = log
        .locator("[data-index]")
        .filter({ hasText: own.content });
      const edit = ownedRow.getByRole("button", {
        name: "Nachricht bearbeiten",
        exact: true,
      });
      await edit.focus();
      await page.keyboard.press("Enter");
      const input = ownedRow.locator("textarea");
      await until(() => focused(input), Boolean, "own-message-edit-focus");
      const editTop = await log.evaluate((el) => el.scrollTop);
      await page.keyboard.press("End");
      await page.keyboard.press("PageDown");
      assert.equal(
        await log.evaluate((el) => el.scrollTop),
        editTop,
        "editing keys jumped canonical history",
      );
      await page.keyboard.press("Escape");
      await capture("12b-edit-escape");
      assert.ok(await focused(edit), "edit Escape lost opener focus");
      await page.keyboard.press("PageDown");
      await until(
        () => log.evaluate((el) => el.scrollTop),
        (top) => top > editTop + 50,
        "native-history-pagedown",
      );
      const visibleAdd = log
        .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
        .last();
      await visibleAdd.focus();
      await page.keyboard.press("End");
      await until(
        () =>
          log.evaluate(
            (el) => el.scrollHeight - el.scrollTop - el.clientHeight,
          ),
        (gap) => gap <= 16,
        "native-history-end",
      );
      await capture("12c-history-end");
    },
  );

  await check(
    "real local call dock remains reachable across chat and short-height navigation",
    async () => {
      await page.goto(`${base}${voicePath}`);
      const join = page.getByRole("button", { name: "Beitreten", exact: true });
      await join.waitFor();
      await join.focus();
      await page.keyboard.press("Enter");
      const dock = page.getByRole("region", {
        name: "Aktive Medien",
        exact: true,
      });
      await dock
        .getByRole("link", { name: "Verbunden: keyboard-call", exact: true })
        .waitFor();
      // SPA navigation preserves the actual call and its dock ownership.
      await page.setViewportSize({ width: 1280, height: 720 });
      await page.locator(`a[href="${chatPath}"]`).first().click();
      await log.waitFor();
      await matrix("13-call-dock", () => dock.waitFor());
      await page.setViewportSize({ width: 390, height: 400 });
      const composer = page.locator("form textarea").last();
      await composer.focus();
      await page.keyboard.type("Call dock keyboard send");
      const composerState = await capture("13b-call-composer-focus");
      assert.ok(
        composerState.composer.bottom <= composerState.dock.y,
        `focused composer is covered by dock: ${composerState.composer.bottom}>${composerState.dock.y}`,
      );
      await page.keyboard.press("Enter");
      await until(
        async () =>
          (
            await api(member, `/channels/${textId}/messages`)
          ).body.messages.some(
            (row) => row.content === "Call dock keyboard send",
          ),
        Boolean,
        "native-composer-send-with-call-dock",
      );
      assert.ok(
        (await log.evaluate((el) => el.clientHeight)) <= 16,
        "fixture must have a clipped canonical viewport",
      );
      const unreadBefore = (await api(member, "/messages/unread")).body.find(
        (row) => row.channel_id === textId,
      ).unread_count;
      assert.equal(
        (
          await api(owner, `/channels/${textId}/messages`, "POST", {
            content: "Clipped viewport incoming message",
          })
        ).status,
        201,
      );
      await until(
        async () =>
          (await api(member, "/messages/unread")).body.find(
            (row) => row.channel_id === textId,
          ).unread_count,
        (count) => count === unreadBefore + 1,
        "clipped-message-viewport-remains-unread",
      );
      await composer.fill("unsent short-height draft");
      const searchTrigger = page.getByRole("button", {
        name: "Nachrichten suchen",
        exact: true,
      });
      await searchTrigger.focus();
      await page.keyboard.press("Enter");
      const searchInput = page.getByRole("searchbox");
      await searchInput.fill("Call dock keyboard send");
      await page.keyboard.press("Enter");
      const hit = page.getByRole("button", {
        name: "Zur Nachricht von A11y member: Call dock keyboard send",
        exact: true,
      });
      await hit.waitFor();
      await hit.focus();
      await page.keyboard.press("Enter");
      const context = page.getByRole("region", {
        name: "Nachrichtenkontext",
        exact: true,
      });
      await context
        .getByText("Call dock keyboard send", { exact: true })
        .waitFor();
      await capture("13c-call-context-short");
      assert.ok(
        await context
          .locator("[data-message-id]")
          .first()
          .evaluate((el) => el.parentElement.clientHeight >= 40),
        "short-height call context has no readable message viewport",
      );
      await context
        .getByRole("button", { name: "Zurück zum Chat", exact: true })
        .focus();
      await page.keyboard.press("Enter");
      assert.ok(await focused(searchTrigger));
      assert.equal(await composer.inputValue(), "unsent short-height draft");
      assert.equal(
        (await api(member, "/messages/unread")).body.find(
          (row) => row.channel_id === textId,
        ).unread_count,
        unreadBefore + 1,
      );
      const mute = dock.getByRole("button", {
        name: "Mikrofon aus",
        exact: true,
      });
      await mute.focus();
      await page.keyboard.press("Enter");
      assert.equal(
        await dock
          .getByRole("button", { name: "Mikrofon an", exact: true })
          .getAttribute("aria-pressed"),
        "true",
      );
      await page.keyboard.press("Enter");
      const settings = dock.getByRole("button", {
        name: "Voice-Einstellungen",
        exact: true,
      });
      await tabTo(settings);
      await page.keyboard.press("Enter");
      const dialog = page.getByRole("dialog", {
        name: "Voice & Video",
        exact: true,
      });
      await dialog.waitFor();
      await capture("14-call-settings-short");
      await page.keyboard.press("Escape");
      assert.ok(await focused(settings));
      const leave = dock.getByRole("button", {
        name: "Verlassen",
        exact: true,
      });
      await tabTo(leave);
      await capture("15-call-leave-focus");
      await page.keyboard.press("Enter");
      await dock.waitFor({ state: "hidden" });
    },
  );
  assert.deepEqual(errors, [], "uncaught browser errors");
} finally {
  if (member) {
    await member.page
      .evaluate(async () => {
        const { leaveVoice } = await import("/src/voice/session.ts");
        await leaveVoice();
      })
      .catch(() => {});
  }
  if (server && owner)
    await api(owner, `/servers/${server}`, "DELETE").catch(() => {});
  for (const context of contexts) await context.close();
  await browser.close();
  await writeFile(
    `${output}/report.json`,
    JSON.stringify(
      {
        base,
        engine,
        version,
        checks,
        failures,
        errors,
        screenshots,
        limits: [
          `local headless ${engine}; fake microphone/camera capture`,
          "viewports model widths, rotation and reduced height; physical iOS and Omarchy keyboard/suspend remain untested",
          "OS notification permission is outside this acceptance",
        ],
      },
      null,
      2,
    ),
  );
}
assert.deepEqual(failures, []);
