/* global URL, Event */
import { readFile } from "node:fs/promises";
import {
  api,
  check,
  click,
  navigate,
  observe,
  snapshot,
  until,
} from "./harness.mjs";

function latch() {
  let release;
  const promise = new Promise((r) => {
    release = r;
  });
  return { promise, release };
}
const log = (a) => a.page.getByRole("log", { name: "Nachrichten" });
const row = (a, text) =>
  log(a).locator("div.group").filter({ hasText: text }).first();
const alert = (a, text) => a.page.getByRole("alert").filter({ hasText: text });
async function send(a, text) {
  await a.page.locator("form textarea").last().fill(text);
  await click(a, "Senden");
}
async function present(a, text) {
  // The UI appends an edited marker inside the content paragraph. Match the
  // complete content, allowing only that known marker, never a text prefix.
  const escaped = text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  await log(a)
    .locator("p")
    .filter({ hasText: new RegExp(`^${escaped}(?:\\s*\\(bearbeitet\\))?$`) })
    .waitFor();
}
async function fresh(f, name) {
  await navigate(f.owner, f.textPath, f.base);
  const path = await f.channel(name, "Text");
  const id = path.split("/").at(-1);
  for (const a of [f.owner, f.member]) await navigate(a, path, f.base);
  return { path, id, endpoint: `**/api/channels/${id}/messages*` };
}
async function messages(a, id) {
  const r = await api(a, `/channels/${id}/messages`);
  check(r.status === 200, "fixture-history-contract-failed", {
    status: r.status,
  });
  return r.body.messages;
}
async function seed(a, id, content) {
  const r = await api(a, `/channels/${id}/messages`, "POST", { content });
  check(r.status === 201, "fixture-message-seed-failed", { status: r.status });
  return r.body;
}

export async function chatFaultScenarios(h, f) {
  await h.run("two-parallel-sends-navigation-retry", ["07", "10"], async () => {
    const c = await fresh(f, "E2E parallel attempts");
    const hold = latch();
    let posts = 0,
      held = false;
    const bytes = await readFile(
      new URL("../fixtures/smoke.png", import.meta.url),
    );
    const text = "E2E parallel failed file",
      success = "E2E parallel successful text";
    await f.owner.page.route(c.endpoint, async (route) => {
      if (route.request().method() === "POST") {
        posts++;
        if (route.request().postDataJSON().content === text && posts === 1) {
          held = true;
          await hold.promise;
          return route.fulfill({
            status: 429,
            contentType: "application/json",
            body: '{"error":"rate_limited"}',
          });
        }
      }
      await route.continue();
    });
    try {
      await f.owner.page.locator('input[type="file"]').setInputFiles({
        name: "parallel.png",
        mimeType: "image/png",
        buffer: bytes,
      });
      check(
        await f.owner.page
          .locator("form textarea")
          .last()
          .evaluate((el) => el === el.ownerDocument.activeElement),
        "attachment-picker-did-not-focus-composer",
      );
      await send(f.owner, text);
      await until(async () => held, Boolean, "first-send-not-held");
      await send(f.owner, success);
      await present(f.member, success);
      await f.owner.page
        .getByRole("link", { name: "E2E Voice B", exact: true })
        .click();
      hold.release();
      await f.owner.page.locator(`a[href="${c.path}"]`).click();
      const failed = alert(f.owner, text);
      await failed
        .getByRole("button", { name: "Sendung wiederholen", exact: true })
        .waitFor();
      check(
        (await failed.getByText("parallel.png", { exact: true }).count()) === 1,
        "failed-attempt-lost-file",
      );
      const before = await messages(f.member, c.id);
      check(
        posts === 2 &&
          before.filter((m) => m.content === success).length === 1 &&
          !before.some((m) => m.content === text),
        "parallel-control-invalid",
        { posts },
      );
      await f.owner.page.unroute(c.endpoint);
      await failed
        .getByRole("button", { name: "Sendung wiederholen", exact: true })
        .click();
      await present(f.member, text);
      const after = await messages(f.member, c.id);
      check(
        after.filter((m) => m.content === text).length === 1 &&
          after.filter((m) => m.content === success).length === 1 &&
          after.find((m) => m.content === text).attachments.length === 1,
        "parallel-retry-lost-or-duplicated-attempt",
        { controlConfirmed: true },
      );
      const image = log(f.member).getByRole("img", {
        name: "parallel.png",
        exact: true,
      });
      await image.waitFor();
      const downloaded = await f.member.context.request.get(
        new URL(await image.getAttribute("src"), f.base).href,
      );
      check(
        downloaded.status() === 200 && (await downloaded.body()).equals(bytes),
        "retry-attachment-bytes-mismatch",
      );
      return {
        overlappingPosts: 2,
        firstStatus: 429,
        secondDeliveredBeforeRelease: true,
        navigation: "SPA",
        retainedTextAndFile: true,
        exactRetryMessages: 1,
        byteEqual: true,
        pr112Focus: true,
      };
    } finally {
      hold.release();
      await f.owner.page.unroute(c.endpoint);
    }
  });

  await h.run(
    "committed-send-gateway-timeout-no-duplicate-retry",
    ["07"],
    async () => {
      const c = await fresh(f, "E2E ambiguous send");
      let posts = 0,
        committed = false;
      const text = "E2E persisted behind proxy504";
      await f.owner.page.route(c.endpoint, async (route) => {
        if (route.request().method() !== "POST") return route.continue();
        posts++;
        const response = await route.fetch();
        committed = response.status() === 201;
        await route.fulfill({
          status: 504,
          contentType: "text/html",
          body: "Synthetic gateway timeout",
        });
      });
      try {
        await send(f.owner, text);
        await present(f.member, text);
        const uncertain = alert(f.owner, text);
        await uncertain
          .getByText("Die Nachricht kann bereits gespeichert sein.", {
            exact: false,
          })
          .waitFor();
        await observe(2_000, async () => ({ posts }));
        const persisted = await messages(f.member, c.id);
        check(
          committed &&
            posts === 1 &&
            persisted.filter((m) => m.content === text).length === 1 &&
            (await uncertain
              .getByRole("button", { name: "Sendung wiederholen", exact: true })
              .count()) === 0,
          "ambiguous-commit-offered-or-performed-duplicate-retry",
          { controlConfirmed: committed, posts },
        );
        await uncertain
          .getByRole("button", { name: "Verwerfen", exact: true })
          .click();
        return {
          upstreamStatus: 201,
          browserStatus: 504,
          persistedMessages: 1,
          postRequests: posts,
          uncertainVisible: true,
          noRetryOffered: true,
        };
      } finally {
        await f.owner.page.unroute(c.endpoint);
      }
    },
  );

  await h.run(
    "edit-delete-failure-row-scoped-rollback",
    ["07", "06b"],
    async () => {
      const controls = [];
      for (const method of ["PATCH", "DELETE"]) {
        const c = await fresh(f, `E2E ${method} rollback`);
        const original = `E2E ${method} original`,
          foreign = `E2E ${method} foreign event`;
        const saved = await seed(f.owner, c.id, original);
        await present(f.owner, original);
        const held = latch();
        const replied = latch();
        let entered = false;
        let routeError = false;
        const endpoint = `**/api/messages/${saved.id}`;
        await f.owner.page.route(endpoint, async (route) => {
          if (route.request().method() !== method) return route.continue();
          entered = true;
          await held.promise;
          try {
            await route.fulfill({
              status: 429,
              contentType: "application/json",
              body: '{"error":"rate_limited"}',
            });
          } catch {
            routeError = true;
          } finally {
            replied.release();
          }
        });
        try {
          if (method === "PATCH") {
            // Message actions take pointer events only on a hovered row.
            await row(f.owner, original).hover();
            await row(f.owner, original)
              .getByRole("button", {
                name: "Nachricht bearbeiten",
                exact: true,
              })
              .click();
            await log(f.owner).locator("textarea").fill("E2E optimistic edit");
            await click(f.owner, "Speichern");
            await present(f.owner, "E2E optimistic edit");
          } else {
            f.owner.page.once("dialog", (d) => d.accept());
            await row(f.owner, original).hover();
            await row(f.owner, original)
              .getByRole("button", { name: "Nachricht löschen", exact: true })
              .click();
            await log(f.owner)
              .getByText(original, { exact: true })
              .waitFor({ state: "hidden" });
          }
          await until(async () => entered, Boolean, "mutation-fault-not-held");
          await seed(f.member, c.id, foreign);
          await present(f.owner, foreign); // positive native WS control before rollback
          held.release();
          await replied.promise;
          check(!routeError, "fixture-mutation-fault-response-not-delivered");
          await present(f.owner, original);
          check(
            (await log(f.owner).getByText(foreign, { exact: true }).count()) ===
              1,
            "row-rollback-overwrote-foreign-event",
            { controlConfirmed: true, method },
          );
          const rest = await messages(f.member, c.id);
          check(
            rest.some((m) => m.content === original) &&
              rest.some((m) => m.content === foreign),
            "row-rollback-rest-control-missing",
          );
          controls.push({
            method,
            faultStatus: 429,
            foreignEventBeforeRollback: true,
            originalRestored: true,
            foreignRetained: true,
          });
        } finally {
          held.release();
          if (entered) await replied.promise;
          await f.owner.page.unroute(endpoint);
        }
      }
      return { controls };
    },
  );

  await h.run(
    "rest-ws-reordering-edit-delete-rollback",
    ["06b", "07"],
    async () => {
      const c = await fresh(f, "E2E delayed REST");
      const edited = await seed(f.owner, c.id, "E2E old REST edit");
      const deleted = await seed(f.owner, c.id, "E2E old REST delete");
      const held = latch();
      const replied = latch();
      let entered = false,
        gets = 0,
        oldRows = false,
        routeError = false;
      await f.member.page.route(c.endpoint, async (route) => {
        if (route.request().method() !== "GET") return route.continue();
        gets++;
        // Hold one genuinely old response. Reconnect/resync may legitimately
        // fetch another page; those responses must remain real and unmodified.
        if (gets !== 1) return route.continue();
        const response = await route.fetch();
        const page = await response.json();
        oldRows =
          page.messages.some((m) => m.id === edited.id) &&
          page.messages.some((m) => m.id === deleted.id);
        entered = true;
        await held.promise;
        try {
          await route.fulfill({ response });
        } catch {
          routeError = true;
        } finally {
          replied.release();
        }
      });
      try {
        await navigate(f.member, c.path, f.base);
        await until(async () => entered, Boolean, "history-response-not-held");
        check(oldRows, "fixture-delayed-rest-missing-old-rows");
        const before = await snapshot(f.member);
        check(
          (
            await api(f.owner, `/messages/${edited.id}`, "PATCH", {
              content: "E2E newer WS edit",
            })
          ).status === 200,
          "fixture-edit-contract-failed",
        );
        check(
          (await api(f.owner, `/messages/${deleted.id}`, "DELETE")).status ===
            204,
          "fixture-delete-contract-failed",
        );
        await seed(f.owner, c.id, "E2E newer WS create");
        const count = (s) =>
          s.sockets
            .filter((x) => x.plane === "gateway")
            .reduce((n, x) => n + x.receivedEvents, 0);
        await until(
          () => snapshot(f.member),
          (s) => count(s) >= count(before) + 3,
          "fixture-native-events-not-delivered",
        );
        held.release();
        await replied.promise;
        check(!routeError, "fixture-delayed-rest-response-not-delivered");
        await present(f.member, "E2E newer WS edit");
        await present(f.member, "E2E newer WS create");
        check(
          (await log(f.member)
            .getByText("E2E old REST delete", { exact: true })
            .count()) === 0 &&
            (await log(f.member)
              .getByText("E2E old REST edit", { exact: true })
              .count()) === 0 &&
            oldRows,
          "late-rest-lost-newer-event-or-resurrected-delete",
          { controlConfirmed: true, historyRequests: gets },
        );
        return {
          oldRowsInHeldResponse: 2,
          nativeEventsBeforeResponse: 3,
          newerEditRetained: true,
          newerCreateRetained: true,
          deleteNotResurrected: true,
          historyRequests: gets,
        };
      } finally {
        held.release();
        if (entered) await replied.promise;
        await f.member.page.unroute(c.endpoint);
      }
    },
  );

  await h.run("paging-error-data-preservation", ["07", "06a"], async () => {
    const c = await fresh(f, "E2E paging failure");
    for (let i = 0; i < 55; i++)
      await seed(f.member, c.id, `E2E page ${String(i).padStart(2, "0")}`);
    await navigate(f.owner, c.path, f.base);
    await present(f.owner, "E2E page 54");
    let requests = 0,
      failing = true;
    await f.owner.page.route(c.endpoint, async (route) => {
      if (
        route.request().method() === "GET" &&
        new URL(route.request().url()).searchParams.has("before")
      ) {
        requests++;
        if (failing)
          return route.fulfill({
            status: 503,
            contentType: "application/json",
            body: '{"error":"unavailable"}',
          });
      }
      await route.continue();
    });
    try {
      await log(f.owner).evaluate((el) => {
        el.scrollTop = 0;
        el.dispatchEvent(new Event("scroll"));
      });
      const retry = log(f.owner).getByRole("button", {
        name: "Erneut laden",
        exact: true,
      });
      await retry.waitFor();
      await observe(3_000, async () => ({ requests }));
      check(
        requests === 1 &&
          (await log(f.owner)
            .getByText("E2E page 05", { exact: true })
            .count()) === 1,
        "paging-failure-erased-data-or-auto-looped",
        { requests },
      );
      failing = false;
      await retry.click();
      await present(f.owner, "E2E page 00");
      check(requests === 2, "paging-explicit-retry-did-not-converge", {
        requests,
        controlConfirmed: true,
      });
      return {
        messagesSeeded: 55,
        initialPagePreserved: true,
        requestsBeforeExplicitRetry: 1,
        requestsAfterRetry: requests,
        oldestVisible: true,
      };
    } finally {
      await f.owner.page.unroute(c.endpoint);
    }
  });

  await h.run(
    "upload-abort-retains-text-file-retry",
    ["07", "10"],
    async () => {
      const c = await fresh(f, "E2E upload abort");
      const bytes = await readFile(
        new URL("../fixtures/smoke.png", import.meta.url),
      );
      let puts = 0,
        binds = 0;
      const endpoint = "**/*";
      await f.owner.page.route(endpoint, async (route) => {
        if (route.request().method() === "PUT") {
          puts++;
          return route.abort("failed");
        }
        if (
          route.request().method() === "POST" &&
          new URL(route.request().url()).pathname ===
            `/api/channels/${c.id}/messages`
        )
          binds++;
        await route.continue();
      });
      try {
        await f.owner.page.locator('input[type="file"]').setInputFiles({
          name: "abort.png",
          mimeType: "image/png",
          buffer: bytes,
        });
        await send(f.owner, "E2E aborted upload text");
        const failed = alert(f.owner, "E2E aborted upload text");
        await failed
          .getByRole("button", { name: "Sendung wiederholen", exact: true })
          .waitFor();
        check(
          puts === 1 &&
            binds === 0 &&
            (await failed.getByText("abort.png", { exact: true }).count()) ===
              1,
          "upload-abort-lost-draft-or-bound-before-put",
          { puts, binds },
        );
        await f.owner.page.unroute(endpoint);
        await failed
          .getByRole("button", { name: "Sendung wiederholen", exact: true })
          .click();
        await present(f.member, "E2E aborted upload text");
        const rest = await messages(f.member, c.id);
        check(
          rest.filter((m) => m.content === "E2E aborted upload text").length ===
            1 &&
            rest.find((m) => m.content === "E2E aborted upload text")
              .attachments.length === 1,
          "upload-retry-lost-or-duplicated",
          { controlConfirmed: true },
        );
        return {
          abortedPuts: puts,
          bindBeforeSuccessfulPut: binds,
          retainedTextAndFile: true,
          exactRetryMessages: 1,
        };
      } finally {
        await f.owner.page.unroute(endpoint);
      }
    },
  );
  await h.run(
    "upload-account-switch-no-foreign-bind",
    ["04", "07", "10"],
    async () => {
      const c = await fresh(f, "E2E upload scope boundary");
      const bytes = await readFile(
        new URL("../fixtures/smoke.png", import.meta.url),
      );
      const held = latch();
      let uploaded = false,
        binds = 0;
      const endpoint = "**/*";
      const text = "E2E old account upload must not reach B";
      await f.owner.page.route(endpoint, async (route) => {
        if (
          route.request().method() === "POST" &&
          new URL(route.request().url()).pathname ===
            `/api/channels/${c.id}/messages`
        )
          binds++;
        if (route.request().method() !== "PUT") return route.continue();
        const response = await route.fetch();
        uploaded = response.status() === 200;
        await held.promise;
        // Scope change aborts the browser fetch. This response is allowed to be canceled.
        await route.fulfill({ response }).catch(() => {});
      });
      try {
        await f.owner.page.locator('input[type="file"]').setInputFiles({
          name: "scope.png",
          mimeType: "image/png",
          buffer: bytes,
        });
        await send(f.owner, text);
        await until(
          async () => uploaded,
          Boolean,
          "fixture-upload-not-committed-before-scope-switch",
        );
        const logout = f.owner.page.waitForResponse(
          (r) =>
            new URL(r.url()).pathname === "/api/auth/logout" &&
            r.request().method() === "POST",
        );
        await click(f.owner, "Abmelden");
        check((await logout).status() === 200, "fixture-logout-failed");
        await f.owner.page.waitForURL(/\/login/);
        await f.owner.page.getByLabel("E-Mail-Adresse").fill(f.member.email);
        await f.owner.page
          .getByLabel("Passwort", { exact: true })
          .fill(f.member.password);
        await click(f.owner, "Anmelden");
        await f.owner.page.waitForURL((u) => !u.pathname.includes("login"));
        check(
          (await api(f.owner, "/auth/session")).body.user?.id === f.member.id,
          "fixture-login-b-failed",
        );
        held.release();
        await observe(1_000, async () => ({ binds }));
        const after = await api(f.owner, "/auth/session");
        check(
          after.body.user?.id === f.member.id &&
            binds === 0 &&
            (await f.owner.page.getByText(text, { exact: true }).count()) ===
              0 &&
            !(await messages(f.member, c.id)).some((m) => m.content === text),
          "old-upload-bound-or-leaked-under-b",
          {
            controlConfirmed: true,
            bindRequests: binds,
            sessionStillB: after.body.user?.id === f.member.id,
          },
        );
        return {
          uploadedBeforeSwitch: true,
          oldBindRequests: binds,
          noAContentsUnderB: true,
          sessionBUnchanged: true,
          aDraftPersistence: "not required across scope change",
        };
      } finally {
        held.release();
        await f.owner.page.unroute(endpoint);
      }
    },
  );
}
