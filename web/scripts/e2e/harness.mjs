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
import {
  NativeInterfaceFailure,
  nativeEvaluate,
  deadlineProbe,
} from "./native-evaluate.mjs";
import { instrument, sample } from "./probe.mjs";
import { selection, wanted, requiredCase, gate } from "./selection.mjs";
import { browserLaunchOptions, iceAdapterOptions } from "./browser-options.mjs";

export class CheckFailure extends Error {
  constructor(code, metrics = {}) {
    super(code);
    this.metrics = metrics;
  }
}
export function check(condition, code, metrics = {}) {
  if (!condition) throw new CheckFailure(code, metrics);
}
export async function pollPause(deadline) {
  // Preserve the 200ms polling cadence. A short final wait completes the
  // observation window; it must not launch another native probe at its edge.
  const nextProbeAt = Date.now() + 200;
  const wakeAt = Math.min(nextProbeAt, deadline);
  while (Date.now() < wakeAt) await pause(Math.max(1, wakeAt - Date.now()));
  return nextProbeAt < deadline && Date.now() < deadline;
}
export async function until(probe, accept, code, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    last = await deadlineProbe(probe, deadline);
    if (Date.now() <= deadline && accept(last)) return last;
    if (!(await pollPause(deadline))) break;
  }
  throw new CheckFailure(code, { last });
}
// Every sample shares this observation deadline; native interface failure is
// distinct from absence/presence of product progress.
export async function observe(duration, probe) {
  const deadline = Date.now() + duration;
  let last;
  while (Date.now() < deadline) {
    last = await deadlineProbe(probe, deadline);
    if (!(await pollPause(deadline))) break;
  }
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
  return error instanceof CheckFailure ||
    error instanceof NativeInterfaceFailure
    ? error.message
    : error?.name === "TimeoutError"
      ? "browser-deadline"
      : "harness-or-interface-error";
}
export async function api(actor, path, method = "GET", body, nativeBudget) {
  const request = async ({ path, method, body }) => {
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
  };
  const args = { path, method, body };
  return nativeBudget === undefined
    ? actor.page.evaluate(request, args)
    : nativeEvaluate(actor, request, args, nativeBudget);
}
export async function startHarness() {
  const base = safeTarget(
    process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174",
  );
  const { engine, relay, firefoxLoopbackIce, sfuLoopbackIce } =
    iceAdapterOptions(process.env);
  const browser = await (engine === "chromium" ? chromium : firefox).launch(
    browserLaunchOptions(engine, { relay, firefoxLoopbackIce, sfuLoopbackIce }),
  );
  const suffix = `${Date.now().toString(36)}-${randomBytes(4).toString("hex")}`;
  const password = randomBytes(24).toString("base64url");
  const selected = selection(process.env.GELABBER_E2E_CASES);
  const actors = [];
  const servers = [];
  const completedActorContextCloses = new WeakSet();
  const testSourceHashes = {};
  for (const filename of (await readdir(new URL("./", import.meta.url)))
    .filter((name) => name.endsWith(".mjs") || name.endsWith(".py"))
    .sort())
    testSourceHashes[filename] = createHash("sha256")
      .update(await readFile(new URL(filename, import.meta.url)))
      .digest("hex");
  const report = {
    testSourceHashes,
    schema: 2,
    selectedCases: selected,
    phase: "11b/c-automation",
    requestedSuite: process.env.GELABBER_E2E_SUITE ?? "all",
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
      sfuLoopbackIceAdapter: {
        enabled: sfuLoopbackIce,
        name: "explicit-own-SFU-loopback-ICE-topology-control",
        scope:
          "local Firefox forced-relay adapter only; no default/WAN acceptance",
        nativeMidIndexOverride: false,
      },
      firefoxLoopbackIceAdapter: {
        enabled: firefoxLoopbackIce,
        pref: "media.peerconnection.ice.loopback",
        value: firefoxLoopbackIce ? true : "browser default",
        scope: "explicit local Firefox forced-relay test profile only",
      },
    },
    limitations: [
      "Automated local/CI subset; not complete 11b/c/d acceptance",
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
    const closeContext = context.close.bind(context);
    context.close = async (...args) => {
      await closeContext(...args);
      completedActorContextCloses.add(context);
    };
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
    // Creating a community now opens its overview. Select the actual text
    // channel through the UI before continuing the message scenarios.
    await owner.page.waitForURL(/\/s\/[^/]+(?:\/c\/[^/]+)?$/);
    const serverId = new URL(owner.page.url()).pathname.split("/")[2];
    servers.push({ owner, id: serverId });
    const detail = await api(owner, `/servers/${serverId}`);
    check(detail.status === 200, "fixture-server-detail-missing");
    const textChannel = detail.body.channels.find(
      (item) => item.kind === "text",
    );
    check(textChannel?.id, "fixture-text-channel-missing");
    const textPath = `/s/${serverId}/c/${textChannel.id}`;
    await owner.page.locator(`a[href="${textPath}"]`).first().click();
    await owner.page.waitForURL((url) => url.pathname === textPath);
    async function channel(name, kind = "Voice") {
      await owner.page
        .locator('summary[aria-label="Kanal oder Kategorie erstellen"]')
        .click();
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
    isolationBlocked = false,
    nativeInterfaceBlocked = false;
  function setIsolation(restore) {
    isolation = restore;
  }
  async function run(id, predecessors, task, { setup = false } = {}) {
    if (!requiredCase(selected, id, setup)) return notRun(id, predecessors);
    if (isolationBlocked || nativeInterfaceBlocked) {
      blocked(
        id,
        nativeInterfaceBlocked
          ? "native-browser-interface-unavailable"
          : "fixture-recovery-failed",
        predecessors,
        { setup },
      );
      return { id, status: "BLOCKED" };
    }
    const at = new Date().toISOString();
    report.checkpoint = {
      case: id,
      stage: "started",
      at,
      completeAcceptance: false,
    };
    const checkpointPath =
      (process.env.GELABBER_E2E_REPORT ?? "/tmp/gelabber-e2e/report.json") +
      ".checkpoint.json";
    await mkdir(dirname(checkpointPath), { recursive: true });
    await writeFile(checkpointPath, JSON.stringify(report) + "\n", {
      mode: 0o600,
    });
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
      if (error instanceof NativeInterfaceFailure)
        nativeInterfaceBlocked = true;
      row = {
        id,
        status: "FAIL",
        reason: safeError(error),
        classification:
          error instanceof NativeInterfaceFailure
            ? "test-error"
            : error instanceof CheckFailure && error.metrics?.controlConfirmed
              ? "confirmed-product-failure"
              : error instanceof CheckFailure && /fixture/.test(error.message)
                ? "test-error"
                : "unconfirmed-failure",
        site:
          /scripts\/e2e\/([a-z.]+:\d+:\d+)/.exec(error.stack ?? "")?.[1] ??
          null,
        ...(error instanceof CheckFailure ||
        error instanceof NativeInterfaceFailure
          ? { metrics: error.metrics }
          : {}),
      };
    }
    if (isolation && !nativeInterfaceBlocked) {
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
  function notRun(id, predecessors = []) {
    const row = {
      id,
      status: "NOT_RUN",
      classification: "not-executed",
      reason: "not-selected-in-this-run",
      predecessors,
      at: new Date().toISOString(),
    };
    report.results.push(row);
    console.log(`NOT_RUN ${id} (${row.reason})`);
    return row;
  }
  function blocked(id, reason, predecessors = [], { setup = false } = {}) {
    if (!requiredCase(selected, id, setup)) return notRun(id, predecessors);
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
      // Scenario finally already closed these owned contexts successfully.
      // An unexpected page closure or rejected context close remains an error.
      if (completedActorContextCloses.has(who.context) && who.page.isClosed())
        continue;
      if (who.nativeEvaluationUnusable) {
        const closed = await deadlineProbe(
          () => who.nativeAbortClose,
          Date.now() + 5_000,
        ).catch(() => false);
        if (closed === false) {
          report.nativeAbortCloseFailed = true;
          process.exitCode = 1;
          report.results.push({
            id: "native-abort-close",
            setup: true,
            status: "FAIL",
            classification: "test-error",
            reason: "native-abort-close-failed",
          });
        }
        continue;
      }
      try {
        await nativeEvaluate(who, () => {
          for (const pc of window.__e2e.peers) pc.close();
          for (const { ws } of window.__e2e.sockets) ws.close();
        });
      } catch (error) {
        report.nativeCloseInterfaceFailed = true;
        report.results.push({
          id: "native-stop-evaluate",
          setup: true,
          status: "FAIL",
          classification: "test-error",
          reason:
            error instanceof NativeInterfaceFailure
              ? "native-stop-interface-failed"
              : "native-stop-evaluate-rejected",
        });
        process.exitCode = 1;
      }
    }
    for (const { owner, id, deleted } of servers) {
      if (deleted) {
        report.cleanup.push({
          target: "owned-test-server",
          status: 204,
          source: "scenario-confirmed-delete",
        });
        continue;
      }
      let stage = "create-cleanup-context",
        context;
      const statuses = {};
      try {
        // A separate login lets logout/revocation scenarios leave the original socket untouched.
        context = await deadlineProbe(
          () => browser.newContext(),
          Date.now() + 12_000,
        );
        const page = await deadlineProbe(
          () => context.newPage(),
          Date.now() + 5_000,
        );
        stage = "load-login";
        await page.goto(`${base}/login`);
        await page.getByLabel("E-Mail-Adresse").fill(owner.email);
        await page.getByLabel("Passwort", { exact: true }).fill(password);
        const login = page.waitForResponse(
          (r) =>
            new URL(r.url()).pathname === "/api/auth/login" &&
            r.request().method() === "POST",
        );
        stage = "submit-login";
        await page
          .getByRole("button", { name: "Anmelden", exact: true })
          .click();
        statuses.login = (await login).status();
        stage = "wait-login-redirect";
        await page.waitForURL((url) => !url.pathname.includes("login"));
        stage = "delete-owned-server";
        const response = await api(
          { page },
          `/servers/${id}`,
          "DELETE",
          undefined,
          10_000,
        );
        statuses.delete = response.status;
        stage = "logout-cleanup-session";
        statuses.logout = (
          await api({ page }, "/auth/logout", "POST", undefined, 10_000)
        ).status;
        check(statuses.logout === 200, "fixture-cleanup-logout-failed");
        report.cleanup.push({
          target: "owned-test-server",
          status: response.status,
          stage: "complete",
          statuses,
        });
      } catch {
        report.cleanup.push({
          target: "owned-test-server",
          status: "FAILED",
          reason: "cleanup-interface-or-deadline",
          stage,
          statuses,
        });
      } finally {
        if (context)
          await deadlineProbe(() => context.close(), Date.now() + 5_000).catch(
            () => {
              process.exitCode = 1;
              report.cleanupContextCloseFailed = true;
              report.results.push({
                id: "cleanup-context-close",
                setup: true,
                status: "FAIL",
                classification: "test-error",
                reason: "cleanup-context-close-failed",
              });
            },
          );
      }
    }
    await deadlineProbe(() => browser.close(), Date.now() + 5_000).catch(() => {
      process.exitCode = 1;
      report.browserCloseFailed = true;
      report.results.push({
        id: "cleanup-browser-close",
        setup: true,
        status: "FAIL",
        classification: "test-error",
        reason: "cleanup-browser-close-failed",
      });
    });
    report.finishedAt = new Date().toISOString();
    for (const id of gate(report.results, selected, report.cleanup).absent)
      blocked(id, "selected-case-unknown-or-not-reached");
    report.gate = gate(report.results, selected, report.cleanup);
    const path =
      process.env.GELABBER_E2E_REPORT ?? "/tmp/gelabber-e2e/report.json";
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, `${JSON.stringify(report, null, 2)}\n`, {
      mode: 0o600,
    });
    if (!report.gate.passed) process.exitCode = 1;
    console.log(
      `Automated gate ${report.gate.passed ? "PASS" : "FAIL"}; redacted report written; complete acceptance remains open.`,
    );
  }
  return {
    actor,
    wants: (id) => wanted(selected, id),
    setup: (id, predecessors, task) =>
      run(id, predecessors, task, { setup: true }),
    fixture,
    markServerDeleted(id) {
      const owned = servers.find((s) => s.id === id);
      check(owned, "fixture-delete-not-owned");
      owned.deleted = true;
    },
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
  return nativeEvaluate(actor, sample);
}
export async function click(actor, name) {
  if (
    name === "Abmelden" &&
    !(await actor.page
      .getByRole("button", { name, exact: true })
      .first()
      .isVisible())
  ) {
    await actor.page.locator('summary[aria-label="Benutzermenü"]').click();
  }
  await actor.page.getByRole("button", { name, exact: true }).first().click();
}
export async function navigate(actor, path, base) {
  await actor.page.goto(`${base}${path}`);
}
