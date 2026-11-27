/* global URL */
import { readFile } from "node:fs/promises";
import {
  api,
  check,
  click,
  confirmDialog,
  navigate,
  observe,
  until,
} from "./harness.mjs";

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
export async function ensureAccount(actor, base) {
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
      const own = f.owner.page
        .getByRole("log", { name: "Nachrichten" })
        .locator(".lr-message-row")
        .filter({ hasText: edited });
      await own.hover();
      await own
        .getByRole("button", { name: "Weitere Aktionen", exact: true })
        .click();
      await f.owner.page
        .getByRole("menuitem", { name: "Löschen", exact: true })
        .click();
      await confirmDialog(f.owner, "Löschen");
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
  // A first DM between people who are not friends is a message request
  // (#563), which takes no files. These scenarios are about an ordinary DM.
  if (
    h.wants("new-dm-connected-recipient-discovers-without-refetch") ||
    h.wants("dm-jpeg-png-webp-bind-reload-byte-equal")
  )
    await h.setup("dm-friends-fixture", [], async () => {
      const asked = await api(f.owner, "/friends/requests", "POST", {
        user_id: f.member.id,
      });
      const answered = await api(f.member, "/friends/requests", "POST", {
        user_id: f.owner.id,
      });
      check(answered.body?.status === "friends", "dm-friends-fixture-failed", {
        asked: asked.status,
        answered: answered.status,
      });
      return { friends: true };
    });
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
        .getByRole("button", { name: "Profil von E2E Member", exact: true })
        .click();
      await f.owner.page
        .getByRole("dialog", { name: "Profil von E2E Member" })
        .getByRole("button", { name: "Nachricht senden", exact: true })
        .click();
      await f.owner.page.waitForURL(/\/d\//);
      f.dmPath = new URL(f.owner.page.url()).pathname;
      await send(f.owner, "E2E synthetic DM first message");
      // No reload, navigation, hover-prefetch or manual list fetch on the receiver.
      const link = f.member.page.locator(
        `.channel-sidebar a[href="${f.dmPath}"]`,
      );
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
  // A first DM from a server member who is no friend is a message request
  // (#563): the connected recipient must see it arrive without a reload.
  await h.run(
    "new-message-request-connected-recipient-discovers-without-refetch",
    ["05a", "06b"],
    async () => {
      // In the app, so the member's gateway socket stays the one it has:
      // a new page would start a socket of its own after the request.
      await f.member.page
        .getByRole("link", { name: "Direktnachrichten", exact: true })
        .click();
      await f.member.page.getByRole("link", { name: /^Anfragen/ }).click();
      await f.member.page
        .getByRole("heading", { name: "Nachrichtenanfragen", exact: true })
        .waitFor();
      const dm = await api(f.watcher, "/dms", "POST", {
        user_id: f.member.id,
      });
      check([200, 201].includes(dm.status), "message-request-dm-failed", {
        status: dm.status,
      });
      const sent = await api(
        f.watcher,
        `/channels/${dm.body.id}/messages`,
        "POST",
        {
          content: "E2E synthetic message request",
        },
      );
      check([200, 201].includes(sent.status), "message-request-send-failed", {
        status: sent.status,
      });
      // No reload, navigation or manual fetch on the recipient.
      const request = f.member.page
        .getByRole("list", { name: "Nachrichtenanfragen" })
        .getByRole("link", { name: "E2E Watch", exact: true });
      let live = true;
      try {
        await request.waitFor({ state: "visible", timeout: 8_000 });
      } catch {
        live = false;
        // A reload is a control probe only: the missed live arrival stays FAIL.
        await f.member.page.reload();
        await request.waitFor({ state: "visible" });
      }
      check(live, "connected-recipient-missed-message-request", {
        controlConfirmed: true,
        discoveredAfterReload: true,
      });
      return { discoveredWithoutReload: true };
    },
  );
  // DM creation can succeed even while discovery is correctly red. Verify attachments independently.
  if (!f.dmPath && h.wants("dm-jpeg-png-webp-bind-reload-byte-equal"))
    await h.setup("dm-attachment-fixture", [], async () => {
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
              .first()
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
            // A picture loads once it is in view (loading="lazy"), and a
            // failed first try mounts a new <img>, so it is looked up afresh
            // each time. Its width says nothing here: with a thumbnail's
            // srcset (#574) the 8 px fixture reports 8 * 8 / 400 = 0 px.
            // decode() settles once the picture is decoded or broken; the
            // bytes are compared below.
            const picture = f.member.page
              .locator(`img[alt="${filename}"]`)
              .last();
            await picture.scrollIntoViewIfNeeded({ timeout: 15_000 });
            await until(
              () =>
                picture.evaluate((img) =>
                  img.decode().then(
                    () => true,
                    () => false,
                  ),
                ),
              Boolean,
              "attachment-not-rendered-after-reload",
              15_000,
            );
            const src = await picture.getAttribute("src");
            // After the reload the picture may show its thumbnail
            // (`?size=400`, made by the media worker); the bytes sent are
            // the original's.
            const original = new URL(src, f.base);
            original.searchParams.delete("size");
            const response = await f.member.page.request.get(original.href);
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
      await f.watcher.page
        .locator(`.channel-sidebar a[href="${path}"]`)
        .click();
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
      const marker = "E2E initial history retry control";
      check(
        (
          await api(f.owner, `/channels/${chat}/messages`, "POST", {
            content: marker,
          })
        ).status === 201,
        "fixture-history-retry-seed-failed",
      );
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
          name: "Erneut laden",
          exact: true,
        });
        const visibleRetry = (await retry.count()) > 0;
        check(
          visibleRetry && failures > 0 && failures <= 4,
          "history-error-hidden-or-retry-unbounded",
          { requests: failures, visibleRetry },
        );
        await f.member.page.unroute(endpoint);
        await retry.first().click();
        await f.member.page
          .locator("form textarea")
          .waitFor({ state: "visible" });
        await visible(f.member, marker);
        return {
          failedRequests: failures,
          explicitRetry: true,
          actualHistoryRestored: true,
        };
      } finally {
        await f.member.page.unroute(endpoint);
      }
    },
  );
  await h.run("failed-send-navigation-retains-own-retry", ["07"], async () => {
    await navigate(f.owner, f.textPath, f.base);
    await navigate(f.member, f.textPath, f.base);
    const endpoint = `**/api/channels/${chat}/messages`;
    let rejected = 0;
    await f.owner.page.route(endpoint, async (route) => {
      if (route.request().method() === "POST") {
        rejected++;
        await route.fulfill({
          status: 429,
          contentType: "application/json",
          body: '{"error":"rate_limited","message":"E2E synthetic send failure"}',
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
        (await failedResponse).status() === 429,
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
      await f.owner.page
        .locator(`.channel-sidebar a[href="${f.textPath}"]`)
        .click();
      const retained = f.owner.page
        .getByRole("log", { name: "Nachrichten" })
        .locator(".lr-message-row.is-failed")
        .filter({ hasText: "E2E recoverable send attempt" });
      const retry = retained.getByRole("button", {
        name: "Erneut senden",
        exact: true,
      });
      await retry.waitFor();
      check((await retry.count()) === 1, "failed-send-lost-on-navigation", {
        rejected,
        retainedAttempt: true,
      });
      await f.owner.page.unroute(endpoint);
      await retry.click();
      await visible(f.member, "E2E recoverable send attempt");
      const history = await api(f.member, `/channels/${chat}/messages`);
      check(
        history.status === 200 &&
          history.body.messages.filter(
            (m) => m.content === "E2E recoverable send attempt",
          ).length === 1,
        "failed-send-explicit-retry-duplicated-or-missing",
        { controlConfirmed: true },
      );
      return { rejected, attemptRetained: true, exactRetryMessages: 1 };
    } finally {
      await f.owner.page.unroute(endpoint);
    }
  });
  const { accountSwitchScenarios } = await import("./account-switch.mjs");
  await accountSwitchScenarios(h, f);
  const { chatFaultScenarios } = await import("./chat-faults.mjs");
  await chatFaultScenarios(h, f);
  const { realtimeFaultScenarios } = await import("./realtime-faults.mjs");
  await realtimeFaultScenarios(h, f, h.faultRuntime);
  const { storageFaultScenarios } = await import("./storage-faults.mjs");
  await storageFaultScenarios(h, f, h.faultRuntime);
  const { gatewayLoadScenarios } = await import("./gateway-load.mjs");
  await gatewayLoadScenarios(h, f, h.faultRuntime);
  const { membershipFaultScenarios } = await import("./membership-faults.mjs");
  await membershipFaultScenarios(h, f, h.faultRuntime);
  h.setIsolation(null);
  h.blocked(
    "attachments-https-wan",
    "local HTTP evidence only; coordinator owns HTTPS/WAN integration",
    ["10"],
  );
}
