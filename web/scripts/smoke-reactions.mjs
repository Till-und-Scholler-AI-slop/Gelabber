// Local real API/browser acceptance; creates disposable accounts and messages.
// Start an isolated migrated API + Vite first. Never point this at production.
/* global process, console, document, window, URL, crypto */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, firefox } from "playwright";
const origin = new URL(
  process.env.GELABBER_REACTIONS_URL ?? "http://127.0.0.1:5173",
);
assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(origin.hostname));
assert.ok(
  !origin.username && !origin.password && !origin.search && !origin.hash,
);
const output =
  process.env.GELABBER_REACTIONS_OUTPUT ?? "/tmp/gelabber-reactions-browser";
await mkdir(output, { recursive: true });
const results = [];
for (const engine of [chromium, firefox]) {
  const browser = await engine.launch();
  try {
    const contexts = await Promise.all(
      [1280, 390, 320].map((width) =>
        browser.newContext({ viewport: { width, height: 844 } }),
      ),
    );
    const pages = await Promise.all(
      contexts.map((context) => context.newPage()),
    );
    const errors = [];
    pages.forEach((page) =>
      page.on("pageerror", (error) => errors.push(error.message)),
    );
    const tokens = new Map();
    async function request(index, method, path, body, expected = 200) {
      const response = await contexts[index].request.fetch(
        new URL(`/api${path}`, origin).href,
        {
          method,
          ...(body !== undefined ? { data: body } : {}),
          headers: tokens.has(index)
            ? { "x-csrf-token": tokens.get(index) }
            : {},
        },
      );
      const value = response.status() === 204 ? null : await response.json();
      assert.equal(
        response.status(),
        expected,
        `${method} ${path}: ${JSON.stringify(value)}`,
      );
      if (value?.csrf_token) tokens.set(index, value.csrf_token);
      return value;
    }
    const nonce = crypto.randomUUID();
    const accounts = [];
    for (let i = 0; i < contexts.length; i++) {
      await request(i, "GET", "/auth/session");
      accounts.push(
        (
          await request(
            i,
            "POST",
            "/auth/register",
            {
              email: `${nonce}-${i}@test.invalid`,
              password: "local-test-password",
              name: `Reactions ${i}`,
            },
            201,
          )
        ).user,
      );
    }
    const server = await request(
      0,
      "POST",
      "/servers",
      { name: "Reaction acceptance" },
      201,
    );
    const channel = server.channels.find(
      (channel) => channel.kind === "text",
    ).id;
    const invite = await request(
      0,
      "POST",
      `/servers/${server.id}/invites`,
      {},
      201,
    );
    await request(1, "POST", `/invites/${invite.code}/join`);
    const message = await request(
      0,
      "POST",
      `/channels/${channel}/messages`,
      { content: `Reaction-${nonce}` },
      201,
    );
    const target = new URL(`/s/${server.id}/c/${channel}`, origin).href;
    await Promise.all(pages.slice(0, 2).map((page) => page.goto(target)));
    const member = pages[1];
    const row = (page) =>
      page.locator(".lr-message-row").filter({ hasText: `Reaction-${nonce}` });
    await row(member)
      .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
      .waitFor();
    await row(member)
      .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
      .click();
    const dialog = member.getByRole("dialog", { name: "Reaktion auswählen" });
    await dialog.getByRole("searchbox").fill("rotes Herz");
    await dialog
      .getByRole("button", { name: "rotes Herz", exact: true })
      .press("Enter");
    await member.waitForFunction(
      () =>
        document
          .querySelector('[aria-label="❤️: 1 Reaktionen, du hast reagiert"]')
          ?.getAttribute("aria-pressed") === "true",
    );
    // Removing the last vote must also survive the Redis cjson [] -> {} shape.
    await row(member)
      .getByRole("button", {
        name: "❤️: 1 Reaktionen, du hast reagiert",
        exact: true,
      })
      .click();
    await row(member)
      .getByRole("button", {
        name: "❤️: 1 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor({ state: "detached" });
    await row(pages[0])
      .getByRole("button", { name: "❤️: 1 Reaktionen", exact: true })
      .waitFor({ state: "detached" });
    const heart = `/messages/${message.id}/reactions/${encodeURIComponent("❤️")}`;
    await request(1, "PUT", heart);
    await row(member)
      .getByRole("button", {
        name: "❤️: 1 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    await request(0, "PUT", heart);
    await row(member)
      .getByRole("button", {
        name: "❤️: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    const thumb = `/messages/${message.id}/reactions/${encodeURIComponent("👍")}`;
    await request(0, "PUT", thumb);
    await row(member)
      .getByRole("button", { name: "👍: 1 Reaktionen", exact: true })
      .click();
    await row(member)
      .getByRole("button", {
        name: "👍: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    const snapshot = (await request(1, "GET", `/channels/${channel}/messages`))
      .messages[0];
    assert.equal(snapshot.content, message.content);
    assert.equal(snapshot.edited_at, null);
    assert.equal(snapshot.reactions.length, 2);
    await row(member)
      .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
      .click();
    await dialog.getByRole("searchbox").waitFor();
    await member.keyboard.press("Escape");
    assert.equal(
      await member.evaluate(() =>
        document.activeElement?.getAttribute("aria-label"),
      ),
      "Reaktion hinzufügen",
    );
    await member.screenshot({
      path: `${output}/${engine.name()}-mobile.png`,
      fullPage: true,
    });
    await member.setViewportSize({ width: 320, height: 520 });
    await row(member)
      .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
      .click();
    await dialog.getByRole("searchbox").fill("Katze");
    assert.equal(
      await member.evaluate(
        () => document.documentElement.scrollWidth > window.innerWidth,
      ),
      false,
    );
    assert.equal(
      await dialog.evaluate(
        (element) => element.scrollWidth > element.clientWidth + 1,
      ),
      false,
    );
    await member.screenshot({
      path: `${output}/${engine.name()}-picker-320.png`,
      fullPage: true,
    });
    await member.keyboard.press("Escape");
    // Remote chips can wrap and resize an already measured virtual row. The
    // reader's visible message must stay put while reading older history.
    const history = await request(
      0,
      "POST",
      `/servers/${server.id}/channels`,
      { name: "reaction-history", kind: "text" },
      201,
    );
    const historyMessages = [];
    for (let i = 0; i < 40; i++) {
      historyMessages.push(
        await request(
          0,
          "POST",
          `/channels/${history.id}/messages`,
          { content: `History-anchor-${i}` },
          201,
        ),
      );
    }
    await member.goto(new URL(`/s/${server.id}/c/${history.id}`, origin).href);
    const log = member.getByRole("log", { name: "Nachrichten" });
    await log
      .locator(".lr-message-row")
      .filter({ hasText: "History-anchor-39" })
      .waitFor();
    const logBox = await log.boundingBox();
    assert.ok(logBox);
    await member.mouse.move(
      logBox.x + logBox.width / 2,
      logBox.y + logBox.height / 2,
    );
    await member.mouse.wheel(0, -400);
    await member.waitForFunction(() => {
      const element = document.querySelector('[role="log"]');
      return (
        element &&
        element.scrollHeight - element.scrollTop - element.clientHeight > 200
      );
    });
    const geometry = await log.evaluate((element) => {
      const top = element.getBoundingClientRect().top;
      const rows = [...element.querySelectorAll(".lr-message-row")].map(
        (row) => ({
          index: Number(
            row
              .querySelector("p")
              ?.textContent?.match(/History-anchor-(\d+)/)?.[1],
          ),
          top: row.getBoundingClientRect().top,
          height: row.getBoundingClientRect().height,
        }),
      );
      return {
        anchor: rows.find((row) => row.top >= top),
        above: rows.filter((row) => row.top + row.height <= top).at(-1),
      };
    });
    assert.ok(
      geometry.anchor && geometry.above,
      "fixture needs a visible anchor and an overscan row above it",
    );
    const anchorRow = log.locator(".lr-message-row").filter({
      has: member.getByText(`History-anchor-${geometry.anchor.index}`, {
        exact: true,
      }),
    });
    const growingRow = log.locator(".lr-message-row").filter({
      has: member.getByText(`History-anchor-${geometry.above.index}`, {
        exact: true,
      }),
    });
    const emojis = [
      "😀",
      "😂",
      "😅",
      "😇",
      "🙂",
      "🙃",
      "😉",
      "😊",
      "😍",
      "🥰",
      "😘",
      "😋",
      "😎",
      "🤔",
      "😴",
      "😮",
      "😢",
      "😡",
      "👍",
      "❤️",
    ];
    for (const emoji of emojis) {
      await request(
        0,
        "PUT",
        `/messages/${historyMessages[geometry.above.index].id}/reactions/${encodeURIComponent(emoji)}`,
      );
    }
    await growingRow
      .getByRole("button", { name: "❤️: 1 Reaktionen", exact: true })
      .waitFor();
    await member.waitForFunction(
      ({ index, oldHeight }) => {
        const row = [...document.querySelectorAll(".lr-message-row")].find(
          (row) =>
            row.querySelector("p")?.textContent === `History-anchor-${index}`,
        );
        return row && row.getBoundingClientRect().height > oldHeight + 30;
      },
      { index: geometry.above.index, oldHeight: geometry.above.height },
    );
    // Wait for the browser's native measurement/scroll adjustment, not a
    // programmatic scrollTo that would hide a jump.
    await member.waitForFunction(
      ({ index, top }) => {
        const row = [...document.querySelectorAll(".lr-message-row")].find(
          (row) =>
            row.querySelector("p")?.textContent === `History-anchor-${index}`,
        );
        return row && Math.abs(row.getBoundingClientRect().top - top) <= 3;
      },
      { index: geometry.anchor.index, top: geometry.anchor.top },
    );
    await member.waitForTimeout(300);
    const finalAnchor = await anchorRow.boundingBox();
    assert.ok(
      finalAnchor && Math.abs(finalAnchor.y - geometry.anchor.top) <= 3,
      "visible history anchor must remain stable after ResizeObserver settles",
    );
    assert.ok(
      await log.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight > 200,
      ),
    );
    await member.screenshot({
      path: `${output}/${engine.name()}-history-anchor.png`,
      fullPage: true,
    });
    await member.goto(target);
    await row(member)
      .getByRole("button", {
        name: "❤️: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    await request(0, "PATCH", `/messages/${message.id}`, {
      content: `Reaction-${nonce} edited`,
    });
    await row(member)
      .getByText(`Reaction-${nonce} edited`, { exact: false })
      .waitFor();
    await row(member)
      .getByRole("button", {
        name: "❤️: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    await request(0, "PATCH", `/servers/${server.id}`, {
      member_permissions: ["send_files", "join_voice"],
    });
    await member.reload();
    await row(member)
      .getByRole("button", {
        name: "❤️: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    assert.equal(
      await row(member)
        .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
        .count(),
      0,
    );
    await row(member)
      .getByRole("button", {
        name: "❤️: 2 Reaktionen, du hast reagiert",
        exact: true,
      })
      .press("Enter");
    await row(member)
      .getByRole("button", { name: "❤️: 1 Reaktionen", exact: true })
      .waitFor();
    assert.equal(
      await row(member)
        .getByRole("button", { name: "❤️: 1 Reaktionen", exact: true })
        .isDisabled(),
      true,
    );
    await request(0, "DELETE", `/messages/${message.id}`, undefined, 204);
    await row(member).waitFor({ state: "detached" });
    const dm = await request(
      0,
      "POST",
      "/dms",
      { user_id: accounts[1].id },
      201,
    );
    const dmMessage = await request(
      0,
      "POST",
      `/channels/${dm.id}/messages`,
      { content: "DM reaction" },
      201,
    );
    await member.goto(new URL(`/d/${dm.id}`, origin).href);
    const dmRow = member
      .locator(".lr-message-row")
      .filter({ hasText: "DM reaction" });
    await dmRow
      .getByRole("button", { name: "Reaktion hinzufügen", exact: true })
      .click();
    await member.getByRole("dialog").getByRole("searchbox").fill("Daumen hoch");
    await member
      .getByRole("dialog")
      .getByRole("button", { name: "Daumen hoch", exact: true })
      .click();
    await dmRow
      .getByRole("button", {
        name: "👍: 1 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    const foreign = await contexts[2].request.put(
      new URL(
        `/api/messages/${dmMessage.id}/reactions/${encodeURIComponent("👍")}`,
        origin,
      ).href,
      { headers: { "x-csrf-token": tokens.get(2) } },
    );
    assert.equal(foreign.status(), 404);
    await member.reload();
    await dmRow
      .getByRole("button", {
        name: "👍: 1 Reaktionen, du hast reagiert",
        exact: true,
      })
      .waitFor();
    assert.deepEqual(errors, []);
    results.push({
      browser: engine.name(),
      status: "passed",
      checks: [
        "multi-emoji",
        "counts + own",
        "WS edit/delete",
        "read-only removal",
        "DM privacy",
        "reload",
        "keyboard + Escape focus",
        "320px picker",
        "remote wrapping chips preserve the visible history anchor",
      ],
      physicalDevice: false,
    });
    await Promise.all(contexts.map((context) => context.close()));
    console.log(`${engine.name()}: reaction acceptance passed`);
  } finally {
    await browser.close();
  }
}
await writeFile(`${output}/report.json`, JSON.stringify(results, null, 2));
