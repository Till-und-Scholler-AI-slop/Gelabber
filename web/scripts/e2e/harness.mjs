/* global process, console, window, fetch, URL */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
const repoRoot = fileURLToPath(new URL("../../../", import.meta.url));
import { setTimeout as pause } from "node:timers/promises";
import { chromium, firefox } from "playwright";
import { instrument, sample } from "./probe.mjs";

export class CheckFailure extends Error {
  constructor(code, metrics = {}) {
    super(code);
    this.metrics = metrics;
  }
}
export function check(condition, code, metrics = {}) {
  if (!condition) throw new CheckFailure(code, metrics);
}
export async function until(probe, accept, code, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  let last;
  do {
    last = await probe();
    if (accept(last)) return last;
    await pause(200);
  } while (Date.now() < deadline);
  throw new CheckFailure(code, { last });
}
// A bounded observation interval is needed to prove continued progress/absence.
export async function observe(duration, probe) {
  const deadline = Date.now() + duration;
  let last;
  do {
    last = await probe();
    await pause(200);
  } while (Date.now() < deadline);
  return last;
}
export function safeTarget(value) {
  const url = new URL(value);
  assert.ok(["http:", "https:"].includes(url.protocol));
  assert.ok(!url.username && !url.password && !url.search && !url.hash);
  // This runner creates accounts/content. Remote deployment is deliberately outside 11a.
  assert.ok(
    ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname),
    "11a requires a loopback test stack",
  );
  return url.origin;
}
export function revision(value) {
  return /^[a-f0-9]{7,64}$/.test(value ?? "") ? value : "unknown";
}
export function safeError(error) {
  // Playwright errors contain URLs, form values and call logs: never persist them.
  return error instanceof CheckFailure
    ? error.message
    : error?.name === "TimeoutError"
      ? "browser-deadline"
      : "harness-or-interface-error";
}
export async function api(actor, path, method = "GET", body) {
  return actor.page.evaluate(
    async ({ path, method, body }) => {
      const session = await fetch("/api/auth/session").then((r) => r.json());
      const response = await fetch(`/api${path}`, {
        method,
        headers: {
          "Content-Type": "application/json",
          "X-CSRF-Token": session.csrf_token,
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      return {
        status: response.status,
        body: response.status === 204 ? null : await response.json(),
      };
    },
    { path, method, body },
  );
}
export async function startHarness() {
  const base = safeTarget(
    process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
  );
  const engine = process.env.GELABBER_E2E_BROWSER ?? "chromium";
  assert.ok(["chromium", "firefox"].includes(engine));
  const relay = process.env.GELABBER_E2E_NETWORK === "relay";
  const browser = await (engine === "chromium" ? chromium : firefox).launch(
    engine === "chromium"
      ? {
          args: [
            "--use-fake-device-for-media-stream",
            "--use-fake-ui-for-media-stream",
          ],
        }
      : {
          firefoxUserPrefs: {
            "media.navigator.streams.fake": true,
            "media.navigator.permission.disabled": true,
          },
        },
  );
  const suffix = `${Date.now().toString(36)}-${randomBytes(4).toString("hex")}`;
  const password = randomBytes(24).toString("base64url");
  const selected = (process.env.GELABBER_E2E_CASES ?? "")
    .split(",")
    .filter(Boolean);
  assert.ok(selected.every((id) => /^[a-z0-9-]+$/.test(id)));
  const actors = [];
  const servers = [];
  const testSourceHashes = {};
  for (const filename of (await readdir(new URL("./", import.meta.url)))
    .filter((name) => name.endsWith(".mjs"))
    .sort())
    testSourceHashes[filename] = createHash("sha256")
      .update(await readFile(new URL(filename, import.meta.url)))
      .digest("hex");
  const report = {
    testSourceHashes,
    schema: 1,
    selectedCases: selected,
    phase: "11a",
    at: new Date().toISOString(),
    source: execFileSync("git", ["rev-parse", "HEAD"], {
      encoding: "utf8",
    }).trim(),
    testFilesDirty:
      execFileSync(
        "git",
        ["status", "--porcelain", "--", "web/scripts/e2e", "web/package.json"],
        { encoding: "utf8", cwd: repoRoot },
      ).trim().length > 0,
    runtime: {
      attribution: "operator-supplied; not runtime digest verification",
      web: revision(process.env.GELABBER_E2E_WEB_SHA),
      api: revision(process.env.GELABBER_E2E_API_SHA),
      media: revision(process.env.GELABBER_E2E_MEDIA_SHA),
    },
    browser: {
      engine,
      version: browser.version(),
      platform: process.platform,
      node: process.version,
    },
    environment: {
      url: base,
      requestedNetwork: relay ? "relay" : "direct",
      https: base.startsWith("https:"),
      capture: "synthetic changing canvas; fake microphone",
      autoplay: "browser default; no policy override",
    },
    limitations: [
      "Not 11b/c/d acceptance",
      "No native display picker, audible two-device speech, WAN or production test",
      "No SFU publication/task telemetry; client bounds only",
      "Accounts remain as isolated example.test fixtures; owned servers are deleted",
    ],
    results: [],
    cleanup: [],
  };
  async function actor(label) {
    const context = await browser.newContext({
      permissions:
        engine === "chromium" && !label.includes("Watch")
          ? ["camera", "microphone"]
          : [],
    });
    await context.addInitScript(instrument, { relay });
    const page = await context.newPage();
    page.setDefaultTimeout(12_000);
    const email = `e2e-${suffix}-${actors.length}@example.test`;
    const result = { page, context, label, email, password };
    actors.push(result);
    await page.goto(`${base}/register`);
    await page.getByLabel("Name", { exact: true }).fill(`E2E ${label}`);
    await page.getByLabel("E-Mail-Adresse").fill(email);
    await page.getByLabel("Passwort", { exact: true }).fill(password);
    await page
      .getByRole("button", { name: "Registrieren", exact: true })
      .click();
    await page.waitForURL((url) => !url.pathname.includes("register"));
    const session = await api(result, "/auth/session");
    check(session.body?.user?.id, "registration-session-missing");
    result.id = session.body.user.id;
    return result;
  }
  async function fixture() {
    const owner = await actor("Publisher");
    await owner.page
      .getByRole("button", { name: "Server erstellen", exact: true })
      .first()
      .click();
    let dialog = owner.page.getByRole("dialog", { name: "Server erstellen" });
    await dialog.getByLabel("Name", { exact: true }).fill(`E2E ${suffix}`);
    await dialog
      .getByRole("button", { name: "Erstellen", exact: true })
      .click();
    await owner.page.waitForURL(/\/s\/[^/]+\/c\//);
    const textPath = new URL(owner.page.url()).pathname;
    const serverId = textPath.split("/")[2];
    servers.push({ owner, id: serverId });
    async function channel(name, kind = "Voice") {
      await owner.page
        .getByRole("button", { name: "Kanal erstellen", exact: true })
        .click();
      dialog = owner.page.getByRole("dialog", { name: "Kanal erstellen" });
      await dialog.getByText(kind, { exact: true }).click();
      await dialog.getByLabel("Name", { exact: true }).fill(name);
      const createdResponse = owner.page.waitForResponse(
        (r) =>
          new URL(r.url()).pathname === `/api/servers/${serverId}/channels` &&
          r.request().method() === "POST",
      );
      await dialog
        .getByRole("button", { name: "Erstellen", exact: true })
        .click();
      const response = await createdResponse;
      check(
        [200, 201].includes(response.status()),
        "channel-fixture-create-rejected",
        { status: response.status() },
      );
      const created = await response.json();
      const path = `/s/${serverId}/c/${created.id}`;
      await owner.page.locator(`a[href="${path}"]`).click();
      return path;
    }
    const voicePath = await channel("E2E Voice A");
    const otherPath = await channel("E2E Voice B");
    await owner.page
      .getByRole("button", { name: "Leute einladen", exact: true })
      .click();
    dialog = owner.page.getByRole("dialog", { name: /Einladen zu/ });
    await dialog
      .getByRole("button", { name: "Link erstellen", exact: true })
      .click();
    const invite = await dialog
      .getByRole("textbox", { name: "Einladungslink" })
      .inputValue();
    await dialog.getByRole("button", { name: "Fertig", exact: true }).click();
    async function join(who) {
      await who.page.goto(invite);
      await who.page
        .getByRole("button", { name: "Beitreten", exact: true })
        .click();
      await who.page.waitForURL(/\/s\//);
      await who.page.goto(`${base}${voicePath}`);
    }
    const member = await actor("Member");
    const watcher = await actor("Watch");
    await join(member);
    await join(watcher);
    await owner.page.goto(`${base}${voicePath}`);
    return {
      owner,
      member,
      watcher,
      join,
      channel,
      serverId,
      textPath,
      voicePath,
      otherPath,
      base,
    };
  }
  let isolation = null,
    isolationBlocked = false;
  function setIsolation(restore) {
    isolation = restore;
  }
  async function run(id, predecessors, task) {
    if (isolationBlocked) {
      blocked(id, "fixture-recovery-failed", predecessors);
      return { id, status: "BLOCKED" };
    }
    if (
      selected.length &&
      id !== "isolated-app-fixture" &&
      !selected.includes(id)
    ) {
      blocked(id, "not-selected-in-this-run", predecessors);
      return { id, status: "BLOCKED" };
    }
    const at = new Date().toISOString();
    const start = Date.now();
    let row;
    try {
      if (isolation) await isolation();
      row = {
        id,
        status: "PASS",
        classification: "observed-pass",
        metrics: await task(),
      };
    } catch (error) {
      row = {
        id,
        status: "FAIL",
        reason: safeError(error),
        classification:
          error instanceof CheckFailure && error.metrics?.controlConfirmed
            ? "confirmed-product-failure"
            : error instanceof CheckFailure && /fixture/.test(error.message)
              ? "test-error"
              : "unconfirmed-failure",
        site:
          /scripts\/e2e\/([a-z.]+:\d+:\d+)/.exec(error.stack ?? "")?.[1] ??
          null,
        ...(error instanceof CheckFailure ? { metrics: error.metrics } : {}),
      };
    }
    if (isolation) {
      try {
        await isolation();
      } catch {
        isolationBlocked = true;
        row.fixtureRecovery = "BLOCKED";
      }
    }
    row.at = at;
    row.durationMs = Date.now() - start;
    row.predecessors = predecessors;
    report.results.push(row);
    console.log(`${row.status} ${id}${row.reason ? ` (${row.reason})` : ""}`);
    return row;
  }
  function blocked(id, reason, predecessors = []) {
    report.results.push({
      id,
      status: "BLOCKED",
      classification: "blocked",
      reason,
      predecessors,
      at: new Date().toISOString(),
    });
    console.log(`BLOCKED ${id} (${reason})`);
  }
  async function finish() {
    // Stop browser traffic before deleting only the servers created by this run.
    for (const who of actors) {
      await who.page
        .evaluate(() => {
          for (const pc of window.__e2e.peers) pc.close();
          for (const { ws } of window.__e2e.sockets) ws.close();
        })
        .catch(() => {});
    }
    for (const { owner, id } of servers) {
      try {
        // A separate login lets logout/revocation scenarios leave the original socket untouched.
        const context = await browser.newContext();
        const page = await context.newPage();
        await page.goto(`${base}/login`);
        await page.getByLabel("E-Mail-Adresse").fill(owner.email);
        await page.getByLabel("Passwort", { exact: true }).fill(password);
        await page
          .getByRole("button", { name: "Anmelden", exact: true })
          .click();
        await page.waitForURL((url) => !url.pathname.includes("login"));
        const response = await api({ page }, `/servers/${id}`, "DELETE");
        report.cleanup.push({
          target: "owned-test-server",
          status: response.status,
        });
        await api({ page }, "/auth/logout", "POST");
        await context.close();
      } catch {
        report.cleanup.push({
          target: "owned-test-server",
          status: "FAILED",
          reason: "cleanup-interface-or-deadline",
        });
      }
    }
    await browser.close();
    report.finishedAt = new Date().toISOString();
    const path =
      process.env.GELABBER_E2E_REPORT ?? "/tmp/gelabber-e2e/report.json";
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, `${JSON.stringify(report, null, 2)}\n`, {
      mode: 0o600,
    });
    if (
      report.results.some((r) => r.status !== "PASS") ||
      report.cleanup.some((r) => r.status !== 204)
    )
      process.exitCode = 1;
    console.log("11a redacted report written; final acceptance remains open.");
  }
  return {
    actor,
    fixture,
    setIsolation,
    run,
    blocked,
    finish,
    report,
    base,
    relay,
    browser,
    actors,
  };
}
export async function snapshot(actor) {
  return actor.page.evaluate(sample);
}
export async function click(actor, name) {
  await actor.page.getByRole("button", { name, exact: true }).first().click();
}
export async function navigate(actor, path, base) {
  await actor.page.goto(`${base}${path}`);
}
