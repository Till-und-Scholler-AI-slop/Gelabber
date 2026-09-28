/* global document, URL */
import { readFile } from "node:fs/promises";
import { api, check, click, navigate, observe, until } from "./harness.mjs";

async function send(actor, content) {
  await actor.page.locator("form textarea").last().fill(content);
  await click(actor, "Senden");
}
async function visible(actor, text) {
  await actor.page
    .getByRole("log", { name: "Nachrichten" })
    .getByText(text, { exact: false })
    .waitFor({ state: "visible" });
}
async function contentInRest(actor, id, text) {
  const response = await api(actor, `/channels/${id}/messages`);
  check(
    response.status === 200 &&
      response.body.messages.some((m) => m.content === text),
    "message-rest-did-not-converge",
    { status: response.status },
  );
}
async function ensureAccount(actor, base) {
  const current = await api(actor, "/auth/session");
  if (current.body.user?.id === actor.id) return;
  if (current.body.user) await api(actor, "/auth/logout", "POST");
  await navigate(actor, "/login", base);
  await actor.page.getByLabel("E-Mail-Adresse").fill(actor.email);
  await actor.page.getByLabel("Passwort", { exact: true }).fill(actor.password);
  await click(actor, "Anmelden");
  await actor.page.waitForURL((u) => !u.pathname.includes("login"));
  check(
    (await api(actor, "/auth/session")).body.user?.id === actor.id,
    "fixture-account-restoration-failed",
  );
}
export async function coreScenarios(h, f) {
  h.setIsolation(async () => {
    for (const actor of [f.owner, f.member, f.watcher])
      await ensureAccount(actor, f.base);
  });
  await h.run("account-session-reload-login", ["04"], async () => {
    await f.owner.page.reload();
    const first = await api(f.owner, "/auth/session");
    check(first.body.user?.id === f.owner.id, "reload-lost-session");
    const logoutResponse = f.owner.page.waitForResponse(
      (r) =>
        new URL(r.url()).pathname === "/api/auth/logout" &&
        r.request().method() === "POST",
    );
    await click(f.owner, "Abmelden");
    await f.owner.page.waitForURL(/\/login/);
    const response = await logoutResponse;
    check(response.status() === 200, "logout-http-failed", {
      status: response.status(),
    });
    const anonymous = await api(f.owner, "/auth/session");
    check(
      anonymous.status === 200 && anonymous.body.user === null,
      "logout-left-session",
      {
        logoutStatus: response.status(),
        sessionStatus: anonymous.status,
        anonymous: anonymous.body.user === null,
      },
    );
    await f.owner.page.getByLabel("E-Mail-Adresse").fill(f.owner.email);
    await f.owner.page
      .getByLabel("Passwort", { exact: true })
      .fill(f.owner.password);
    await click(f.owner, "Anmelden");
    await f.owner.page.waitForURL((u) => !u.pathname.includes("login"));
    check(
      (await api(f.owner, "/auth/session")).body.user?.id === f.owner.id,
      "login-account-mismatch",
    );
    return {
      reloadSameAccount: true,
      anonymousAfterLogout: true,
      loginSameAccount: true,
    };
  });
  const chat = f.textPath.split("/").at(-1);
  await h.run(
    "channel-chat-create-edit-delete-converges",
    ["05a", "06a"],
    async () => {
      for (const a of [f.owner, f.member])
        await navigate(a, f.textPath, f.base);
      const initial = "E2E synthetic create",
        edited = "E2E synthetic edit";
      await send(f.owner, initial);
      await visible(f.member, initial);
      await contentInRest(f.member, chat, initial);
      await click(f.owner, "Nachricht bearbeiten");
      await f.owner.page
        .locator('[aria-label="Nachrichten"] textarea')
        .fill(edited);
      await click(f.owner, "Speichern");
      await visible(f.member, edited);
      await contentInRest(f.member, chat, edited);
      f.owner.page.once("dialog", (d) => d.accept());
      await click(f.owner, "Nachricht löschen");
      await f.member.page
        .getByRole("log", { name: "Nachrichten" })
        .getByText(edited, { exact: false })
        .waitFor({ state: "hidden" });
      const rest = await api(f.member, `/channels/${chat}/messages`);
      check(
        !rest.body.messages.some((m) => [initial, edited].includes(m.content)),
        "deleted-message-rest-resurrected",
      );
      return {
        create: true,
        edit: true,
        delete: true,
        otherClientWithoutReload: true,
      };
    },
  );
  await h.run(
    "new-dm-connected-recipient-discovers-without-refetch",
    ["05a", "06b"],
    async () => {
      await navigate(f.owner, f.textPath, f.base);
      await f.member.page
        .getByRole("link", { name: "Direktnachrichten", exact: true })
        .click();
      await f.member.page
        .getByRole("complementary", { name: "Direktnachrichten" })
        .waitFor();
      await f.owner.page
        .getByRole("complementary", { name: "Mitglieder", exact: true })
        .locator('button[title="Nachricht an E2E Member"]')
        .click();
      await f.owner.page.waitForURL(/\/d\//);
      f.dmPath = new URL(f.owner.page.url()).pathname;
      await send(f.owner, "E2E synthetic DM first message");
      // No reload, navigation, hover-prefetch or manual list fetch on the receiver.
      const link = f.member.page.locator(`a[href="${f.dmPath}"]`);
      let liveDiscovery = true;
      try {
        await link.waitFor({ state: "visible", timeout: 8_000 });
      } catch {
        liveDiscovery = false;
        // A reload is a control probe only: the missed live discovery remains FAIL.
        await f.member.page.reload();
        await link.waitFor({ state: "visible" });
      }
      await link.click();
      await visible(f.member, "E2E synthetic DM first message");
      check(liveDiscovery, "connected-recipient-missed-dm-discovery", {
        controlConfirmed: true,
        discoveredAfterReload: true,
        firstMessageVisibleAfterReload: true,
      });
      return { discoveredWithoutReload: true, firstMessageVisible: true };
    },
  );
  // DM creation can succeed even while discovery is correctly red. Verify attachments independently.
  if (!f.dmPath)
    await h.run("dm-attachment-fixture", [], async () => {
      const response = await api(f.owner, "/dms", "POST", {
        user_id: f.member.id,
      });
      check(
        response.status === 200 || response.status === 201,
        "dm-fixture-failed",
        { status: response.status },
      );
      f.dmPath = `/d/${response.body.id}`;
      return { status: response.status };
    });
  for (const [destination, path] of [
    ["channel", f.textPath],
    ["dm", f.dmPath],
  ]) {
    if (!path) {
      h.blocked(
        `${destination}-jpeg-png-webp-bind-reload-byte-equal`,
        "dm-fixture-failed",
      );
      continue;
    }
    await h.run(
      `${destination}-jpeg-png-webp-bind-reload-byte-equal`,
      ["07", "10"],
      async () => {
        for (const a of [f.owner, f.member]) await navigate(a, path, f.base);
        const files = [];
        for (const [ext, mimeType] of [
          ["jpg", "image/jpeg"],
          ["png", "image/png"],
          ["webp", "image/webp"],
        ]) {
          const filename = `e2e-${destination}.${ext}`;
          const bytes = await readFile(
            new URL(`../fixtures/smoke.${ext}`, import.meta.url),
          );
          const stages = { presign: 0, put: 0, bind: 0 };
          const onResponse = (response) => {
            const request = response.request(),
              method = request.method();
            const pathname = new URL(response.url()).pathname;
            if (method === "POST" && pathname.endsWith("/attachments"))
              stages.presign = response.status();
            if (method === "PUT") stages.put = response.status();
            if (method === "POST" && pathname.endsWith("/messages"))
              stages.bind = response.status();
          };
          f.owner.page.on("response", onResponse);
          try {
            await f.owner.page
              .locator('input[type="file"]')
              .setInputFiles({ name: filename, mimeType, buffer: bytes });
            await send(
              f.owner,
              `E2E synthetic ${destination} ${ext} attachment`,
            );
            await until(
              async () => ({ ...stages }),
              (s) =>
                s.presign >= 200 &&
                s.presign < 300 &&
                s.put >= 200 &&
                s.put < 300 &&
                s.bind >= 200 &&
                s.bind < 300,
              "attachment-stage-failed",
              15_000,
            );
            await f.member.page.reload();
            await f.member.page.waitForFunction(
              (name) =>
                [...document.images].some(
                  (i) => i.alt === name && i.complete && i.naturalWidth === 8,
                ),
              filename,
              { timeout: 15_000 },
            );
            const src = await f.member.page
              .locator(`img[alt="${filename}"]`)
              .getAttribute("src");
            const response = await f.member.page.request.get(
              new URL(src, f.base).href,
            );
            check(
              response.status() === 200 &&
                (await response.body()).equals(bytes),
              "attachment-download-differs",
              { status: response.status() },
            );
            files.push({
              type: mimeType,
              bytes: bytes.length,
              ...stages,
              renderedAfterReload: true,
              byteEqual: true,
            });
          } finally {
            f.owner.page.off("response", onResponse);
          }
        }
        return {
          files,
          transport: h.report.environment.https
            ? "HTTPS"
            : "HTTP-local-only; HTTPS acceptance BLOCKED",
        };
      },
    );
  }
  await h.run(
    "background-event-before-first-open-55-history-paging",
    ["06a"],
    async () => {
      await navigate(f.owner, f.textPath, f.base);
      const path = await f.channel("E2E History", "Text"),
        id = path.split("/").at(-1);
      // Keep receiver connected elsewhere, and never open this new channel before the events.
      await navigate(f.watcher, f.textPath, f.base);
      for (let i = 0; i < 55; i++) {
        const response = await api(
          f.member,
          `/channels/${id}/messages`,
          "POST",
          {
            content: `E2E history ${i.toString().padStart(2, "0")}`,
            attachment_ids: [],
          },
        );
        check(
          response.status === 201 || response.status === 200,
          "history-fixture-create-failed",
          { index: i, status: response.status },
        );
      }
      // SPA channel open uses the background cache and exposes incomplete-cache regressions.
      await f.watcher.page.locator(`a[href="${path}"]`).click();
      await visible(f.watcher, "E2E history 54");
      const pane = f.watcher.page.getByRole("log", { name: "Nachrichten" });
      const olderRequests = [];
      const onResponse = (response) => {
        const url = new URL(response.url());
        if (
          url.pathname === `/api/channels/${id}/messages` &&
          url.searchParams.has("before")
        )
          olderRequests.push(response.status());
      };
      f.watcher.page.on("response", onResponse);
      try {
        await until(
          async () => {
            await pane.evaluate((el) => {
              el.scrollTop = 0;
            });
            return {
              oldestVisible: await f.watcher.page
                .getByText("E2E history 00", { exact: true })
                .isVisible(),
              olderPages: olderRequests.length,
            };
          },
          (s) => s.oldestVisible && s.olderPages > 0,
          "complete-history-paging-missing",
          10_000,
        );
      } finally {
        f.watcher.page.off("response", onResponse);
      }
      return {
        seeded: 55,
        oldestVisible: true,
        olderPageStatuses: olderRequests,
      };
    },
  );
  await h.run(
    "initial-history-error-visible-bounded-retry",
    ["07"],
    async () => {
      let failures = 0;
      const endpoint = `**/api/channels/${chat}/messages*`;
      await f.member.page.route(endpoint, async (route) => {
        if (route.request().method() === "GET") {
          failures++;
          await route.fulfill({
            status: 503,
            contentType: "application/json",
            body: '{"error":"unavailable","message":"E2E synthetic outage"}',
          });
        } else await route.continue();
      });
      try {
        await navigate(f.member, f.textPath, f.base);
        await observe(6_000, async () => ({ failures }));
        const retry = f.member.page.getByRole("button", {
          name: /Erneut|Wiederholen|Retry/i,
        });
        const visibleRetry = (await retry.count()) > 0;
        check(
          visibleRetry && failures <= 4,
          "history-error-hidden-or-retry-unbounded",
          { requests: failures, visibleRetry },
        );
        await f.member.page.unroute(endpoint);
        await retry.first().click();
        await f.member.page
          .locator("form textarea")
          .waitFor({ state: "visible" });
        return { failedRequests: failures, explicitRetry: true };
      } finally {
        await f.member.page.unroute(endpoint);
      }
    },
  );
  await h.run("failed-send-navigation-retains-own-retry", ["07"], async () => {
    await navigate(f.owner, f.textPath, f.base);
    const endpoint = `**/api/channels/${chat}/messages`;
    let rejected = 0;
    await f.owner.page.route(endpoint, async (route) => {
      if (route.request().method() === "POST") {
        rejected++;
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: '{"error":"unavailable","message":"E2E synthetic send failure"}',
        });
      } else await route.continue();
    });
    try {
      const failedResponse = f.owner.page.waitForResponse(
        (r) =>
          new URL(r.url()).pathname === `/api/channels/${chat}/messages` &&
          r.request().method() === "POST",
      );
      await send(f.owner, "E2E recoverable send attempt");
      check(
        (await failedResponse).status() === 503,
        "send-fault-response-missing",
      );
      await until(
        async () => ({ rejected }),
        (s) => s.rejected === 1,
        "send-fault-not-exercised",
      );
      await f.owner.page
        .getByRole("link", { name: "E2E Voice B", exact: true })
        .click();
      // Navigate by observed href without a reload (draft must survive unmount).
      await f.owner.page.locator(`a[href="${f.textPath}"]`).click();
      const draft = await f.owner.page
        .locator("form textarea")
        .last()
        .inputValue();
      const retry = await f.owner.page
        .getByRole("button", { name: /Erneut senden|Wiederholen/ })
        .count();
      check(
        draft === "E2E recoverable send attempt" || retry > 0,
        "failed-send-lost-on-navigation",
        {
          rejected,
          draftRetained: draft === "E2E recoverable send attempt",
          retryVisible: retry > 0,
        },
      );
      return { rejected, attemptRetained: true };
    } finally {
      await f.owner.page.unroute(endpoint);
    }
  });
  const { accountSwitchScenarios } = await import("./account-switch.mjs");
  await accountSwitchScenarios(h, f);
  h.blocked(
    "rest-ws-reordering-edit-delete-rollback",
    "07 pending; scoped delayed REST/event ordering fixture required",
    ["07"],
  );
  h.blocked(
    "two-parallel-sends-upload-account-switch",
    "07 pending; per-attempt recovery UI contract required",
    ["07"],
  );
  h.blocked(
    "paging-error-data-preservation",
    "07 pending; separate multi-page failed fetch/retry scenario required",
    ["07"],
  );
  h.blocked(
    "redis-epoch-replay-window-reset",
    "shared Redis must not be reset; isolated service failure contract needed",
    ["05a", "06b"],
  );
  h.blocked(
    "storage-delete-outage-durable-cleanup",
    "shared MinIO/DB must not be stopped; read-only own-object cleanup instrumentation needed",
    ["10"],
  );
  h.setIsolation(null);
  h.blocked(
    "attachments-https-wan",
    "local HTTP evidence only; coordinator owns HTTPS/WAN integration",
    ["10"],
  );
}
