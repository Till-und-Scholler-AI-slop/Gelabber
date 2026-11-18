/* global process, console, window, document, navigator, URL, fetch, setInterval, clearInterval */
// Measures what a Live loses when the network gets tight (GEL-26): the
// resolution or the frame rate. One browser goes live with a synthetic
// 1080p "terminal" screen, a second one watches. The script then limits the
// viewer's downlink and the sender's uplink step by step with `tc` on the
// loopback device and records, per step, the size and frame rate the viewer
// receives and the layers the sender still sends.
//
// Needs root (tc), a running stack on loopback (web dev server or preview,
// API, media gateway with MEDIA_ADVERTISED_IP=127.0.0.1) and no other qdisc
// on `lo`. Usage:
//
//   GELABBER_SMOKE_BASE=http://127.0.0.1:5173 node scripts/smoke-screen-sharpness.mjs
//
// Optional: SCREEN_MODE=sharp|smooth ("Text scharf halten" in Settings →
// Video, for a client that has it), LABEL (written into every result line), OUT (a JSON lines
// file), STEP_S (seconds per step), DOWN / UP (comma separated kbit/s).

import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { setTimeout as pause } from "node:timers/promises";
import { chromium } from "playwright";

const base = process.env.GELABBER_SMOKE_BASE ?? "http://127.0.0.1:5173";
const label = process.env.LABEL ?? "run";
const out = process.env.OUT ?? null;
const stepSeconds = Number(process.env.STEP_S ?? 30);
const stepList = (value) => value.split(",").filter(Boolean).map(Number);
const downSteps = stepList(process.env.DOWN ?? "3000,1500,800,400");
const upSteps = stepList(process.env.UP ?? "3000,1500,800,400");
const screenMode = process.env.SCREEN_MODE ?? null;
const SFU_PORT = Number(process.env.SFU_PORT ?? 10000);
const suffix = `${Date.now()}-${process.pid}`;
const password = `Smoke-${suffix}-password`;

function tc(...args) {
  execFileSync("tc", args, { stdio: ["ignore", "ignore", "pipe"] });
}

/** One shaped class per direction; everything else on `lo` passes. */
function shapeStart() {
  tc(
    "qdisc",
    "add",
    "dev",
    "lo",
    "root",
    "handle",
    "1:",
    "htb",
    "default",
    "1",
    "r2q",
    "1000",
  );
  tc(
    "class",
    "add",
    "dev",
    "lo",
    "parent",
    "1:",
    "classid",
    "1:1",
    "htb",
    "rate",
    "40gbit",
  );
}

function shapeStop() {
  try {
    tc("qdisc", "del", "dev", "lo", "root");
  } catch {
    // Nothing to remove.
  }
}

/** Limits UDP from `sport` to any of `dports` (or the other way round). */
function limit(classid, kbit, matches) {
  tc(
    "class",
    "replace",
    "dev",
    "lo",
    "parent",
    "1:",
    "classid",
    `1:${classid}`,
    "htb",
    "rate",
    `${kbit}kbit`,
    "ceil",
    `${kbit}kbit`,
    "burst",
    "15k",
  );
  try {
    tc(
      "qdisc",
      "replace",
      "dev",
      "lo",
      "parent",
      `1:${classid}`,
      "handle",
      `${classid}:`,
      "pfifo",
      "limit",
      "60",
    );
  } catch {
    // Already there.
  }
  try {
    tc("filter", "del", "dev", "lo", "parent", "1:", "prio", String(classid));
  } catch {
    // First step: no filter yet.
  }
  for (const match of matches) {
    tc(
      "filter",
      "add",
      "dev",
      "lo",
      "protocol",
      "ip",
      "parent",
      "1:",
      "prio",
      String(classid),
      "u32",
      ...match,
      "match",
      "ip",
      "protocol",
      "17",
      "0xff",
      "flowid",
      `1:${classid}`,
    );
  }
}

function unlimit(classid) {
  try {
    tc("filter", "del", "dev", "lo", "parent", "1:", "prio", String(classid));
  } catch {
    // No filter.
  }
  try {
    tc("class", "del", "dev", "lo", "classid", `1:${classid}`);
  } catch {
    // No class.
  }
}

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM ?? undefined,
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    "--autoplay-policy=no-user-gesture-required",
  ],
});

const pages = [];

async function participant(name, fakeScreen) {
  const context = await browser.newContext({
    permissions: ["camera", "microphone"],
    viewport: { width: 1600, height: 1000 },
    locale: "de-DE",
  });
  await context.addInitScript(
    (fake) => {
      const NativePeer = window.RTCPeerConnection;
      window.__smokePeers = [];
      window.RTCPeerConnection = class extends NativePeer {
        constructor(config) {
          super(config);
          window.__smokePeers.push(this);
        }
      };
      if (!fake.screen) return;
      // A 1080p "terminal": small text everywhere, one line changes per frame,
      // so every frame costs bits and blur shows in the text.
      navigator.mediaDevices.getDisplayMedia = async () => {
        const canvas = document.createElement("canvas");
        canvas.width = 1920;
        canvas.height = 1080;
        const ctx = canvas.getContext("2d");
        let frame = 0;
        const draw = () => {
          ctx.fillStyle = "#101418";
          ctx.fillRect(0, 0, canvas.width, canvas.height);
          ctx.font = "13px monospace";
          for (let row = 0; row < 66; row += 1) {
            ctx.fillStyle = row % 7 === 0 ? "#9fe870" : "#d0d7de";
            const n = (row * 7919 + Math.floor(frame / 30)) % 100000;
            ctx.fillText(
              `${String(row).padStart(3, "0")} $ cargo test -p gelabber-media sfu::layers::case_${n} ... ok  ${"#".repeat((row * 13 + frame) % 90)}`,
              8,
              15 + row * 16,
            );
          }
          ctx.fillStyle = "#ff7b72";
          ctx.fillRect((frame * 12) % 1920, 1060, 120, 16);
          frame += 1;
        };
        draw();
        const timer = setInterval(draw, 1000 / 30);
        const stream = canvas.captureStream(30);
        const video = stream.getVideoTracks()[0];
        const stopVideo = video.stop.bind(video);
        video.stop = () => {
          clearInterval(timer);
          stopVideo();
        };
        return stream;
      };
    },
    { screen: fakeScreen },
  );
  const page = await context.newPage();
  pages.push(page);
  await page.goto(`${base}/register`);
  await page.getByLabel("Name").fill(name);
  await page
    .getByLabel("E-Mail-Adresse")
    .fill(
      `sharp-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${suffix}@example.test`,
    );
  await page.getByLabel("Passwort", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Registrieren" }).click();
  await page.waitForURL((url) => !url.pathname.includes("register"));
  return { context, page };
}

/** Local UDP ports of the page's connected peers. */
async function localPorts(page) {
  return page.evaluate(async () => {
    const ports = [];
    for (const peer of window.__smokePeers ?? []) {
      if (peer.connectionState !== "connected") continue;
      const report = await peer.getStats();
      const transport = [...report.values()].find(
        (s) => s.type === "transport" && s.selectedCandidatePairId,
      );
      const pair = transport && report.get(transport.selectedCandidatePairId);
      const local = pair && report.get(pair.localCandidateId);
      if (local?.port) ports.push(local.port);
    }
    return ports;
  });
}

async function videoStats(page) {
  return page.evaluate(async () => {
    const rows = [];
    for (const peer of window.__smokePeers ?? []) {
      if (peer.connectionState !== "connected") continue;
      for (const s of (await peer.getStats()).values()) {
        if (s.kind !== "video") continue;
        if (s.type === "inbound-rtp" || s.type === "outbound-rtp")
          rows.push({
            type: s.type,
            rid: s.rid ?? null,
            width: s.frameWidth ?? 0,
            height: s.frameHeight ?? 0,
            fps: s.framesPerSecond ?? 0,
            bytes: s.type === "inbound-rtp" ? s.bytesReceived : s.bytesSent,
            limit: s.qualityLimitationReason ?? null,
            active: s.active ?? null,
            ssrc: s.ssrc,
          });
      }
    }
    return rows;
  });
}

function median(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
}

/** Samples both pages for `seconds`; returns medians of the second half. */
async function sample(a, b, seconds) {
  const samples = [];
  let previous = null;
  for (let i = 0; i < seconds; i += 1) {
    await pause(1000);
    const [sent, received] = await Promise.all([videoStats(a), videoStats(b)]);
    const now = { t: Date.now(), sent, received };
    if (previous) {
      const dt = (now.t - previous.t) / 1000;
      const kbps = (rows, before, type) =>
        rows
          .filter((r) => r.type === type)
          .reduce((sum, r) => {
            const old = before.find((p) => p.ssrc === r.ssrc);
            return sum + (old ? ((r.bytes - old.bytes) * 8) / dt / 1000 : 0);
          }, 0);
      const inbound = received.filter((r) => r.type === "inbound-rtp");
      const top = inbound.sort((x, y) => y.bytes - x.bytes)[0];
      samples.push({
        recvWidth: top?.width ?? 0,
        recvHeight: top?.height ?? 0,
        recvFps: top?.fps ?? 0,
        recvKbps: kbps(received, previous.received, "inbound-rtp"),
        sendKbps: kbps(sent, previous.sent, "outbound-rtp"),
        sendLayers: sent
          .filter((r) => r.type === "outbound-rtp")
          .map(
            (r) =>
              `${r.rid ?? "-"}:${r.width}x${r.height}@${Math.round(r.fps)}${r.limit && r.limit !== "none" ? `(${r.limit})` : ""}`,
          )
          .join(" "),
      });
    }
    previous = now;
  }
  const tail = samples.slice(Math.floor(samples.length / 2));
  return {
    recvHeight: median(tail.map((s) => s.recvHeight)),
    recvFps: median(tail.map((s) => s.recvFps)),
    recvKbps: Math.round(median(tail.map((s) => s.recvKbps)) ?? 0),
    sendKbps: Math.round(median(tail.map((s) => s.sendKbps)) ?? 0),
    sendLayers: tail.at(-1)?.sendLayers ?? "",
  };
}

function record(step, result) {
  const line = { label, mode: screenMode, step, ...result };
  console.log(JSON.stringify(line));
  if (out) appendFileSync(out, JSON.stringify(line) + "\n");
}

shapeStop();
shapeStart();
try {
  const a = await participant("Sender", true);
  await a.page
    .getByRole("button", { name: "Server erstellen" })
    .first()
    .click();
  const serverDialog = a.page.getByRole("dialog", { name: "Server erstellen" });
  await serverDialog.getByLabel("Name").fill(`Sharp ${suffix}`);
  await serverDialog.getByRole("button", { name: "Erstellen" }).click();
  await a.page.waitForURL(/\/s\/[^/]+$/);
  const inviteDialog = a.page.getByRole("dialog", { name: /einladen$/ });
  await inviteDialog.getByRole("button", { name: "Schließen" }).click();
  // Closing the dialog drops its unused link; make one that stays.
  const serverId = new URL(a.page.url()).pathname.split("/")[2];
  const invite = await a.page.evaluate(async (id) => {
    const session = await (await fetch("/api/auth/session")).json();
    const csrf = session.csrf_token;
    const response = await fetch(`/api/servers/${id}/invites`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-CSRF-Token": csrf ?? "",
      },
      body: "{}",
    });
    return (await response.json()).code;
  }, serverId);
  await a.page
    .getByRole("link", { name: /allgemein/ })
    .first()
    .click();
  await a.page.waitForURL(/\/s\/[^/]+\/c\//);
  await a.page.locator("summary.server-sidebar-menu-button").click();
  await a.page.getByRole("button", { name: "Kanal erstellen" }).click();
  const channelDialog = a.page.getByRole("dialog", { name: "Kanal erstellen" });
  await channelDialog.getByText("Sprache", { exact: true }).click();
  await channelDialog.getByLabel("Name").fill("Sharp Voice");
  await channelDialog.getByRole("button", { name: "Erstellen" }).click();
  await a.page.getByRole("link", { name: "Sharp Voice" }).click();
  const voiceUrl = a.page.url();

  const b = await participant("Viewer", false);
  await b.page.goto(`${base}/invite/${invite}`);
  await b.page.getByRole("button", { name: "Beitreten", exact: true }).click();
  await b.page.waitForURL(/\/s\//);
  await b.page.goto(voiceUrl);

  // Screen profile 1080p30, so each layer has 30/15/7.5 FPS steps.
  await a.page
    .locator(".user-panel")
    .getByRole("link", { name: "Einstellungen", exact: true })
    .click();
  await a.page
    .getByRole("navigation", { name: "Einstellungsbereiche" })
    .getByRole("link", { name: "Video", exact: true })
    .click();
  const settings = a.page.locator(".settings-area--video");
  await settings
    .locator("#screen-stream-profile-resolution")
    .selectOption("1080");
  await settings.locator("#screen-stream-profile-fps").selectOption("30");
  if (screenMode) {
    // "Text scharf halten" on for sharp, off for smooth.
    await settings
      .locator("#screen-keep-sharp")
      .setChecked(screenMode === "sharp");
  }
  await a.page.goto(voiceUrl);

  for (const page of [a.page, b.page]) {
    await page
      .locator(".voice-room")
      .getByRole("button", { name: "Beitreten", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Verlassen", exact: true })
      .first()
      .waitFor();
  }
  await a.page
    .getByRole("button", { name: "Live gehen", exact: true })
    .first()
    .click();
  const item = a.page.getByRole("menuitem", {
    name: "Live gehen",
    exact: true,
  });
  if (await item.isVisible().catch(() => false)) await item.click();
  const liveTile = b.page
    .locator("figure")
    .filter({ has: b.page.locator("figcaption.voice-live-caption") });
  await liveTile
    .getByRole("button", { name: "Zuschauen", exact: true })
    .click();
  await b.page.waitForFunction(
    () => [...document.querySelectorAll("video")].some((v) => v.videoWidth > 0),
    null,
    { timeout: 45_000 },
  );
  await pause(8_000);

  const senderPorts = await localPorts(a.page);
  const viewerPorts = await localPorts(b.page);
  console.log(`sender ports ${senderPorts}, viewer ports ${viewerPorts}`);

  record("baseline", await sample(a.page, b.page, stepSeconds));

  // The viewer's downlink: SFU -> viewer.
  for (const kbit of downSteps) {
    limit(
      20,
      kbit,
      viewerPorts.map((port) => [
        "match",
        "ip",
        "sport",
        String(SFU_PORT),
        "0xffff",
        "match",
        "ip",
        "dport",
        String(port),
        "0xffff",
      ]),
    );
    record(`down ${kbit} kbit/s`, await sample(a.page, b.page, stepSeconds));
  }
  unlimit(20);
  record("down recovered", await sample(a.page, b.page, stepSeconds));

  // The sender's uplink: sender -> SFU.
  for (const kbit of upSteps) {
    limit(
      30,
      kbit,
      senderPorts.map((port) => [
        "match",
        "ip",
        "sport",
        String(port),
        "0xffff",
        "match",
        "ip",
        "dport",
        String(SFU_PORT),
        "0xffff",
      ]),
    );
    record(`up ${kbit} kbit/s`, await sample(a.page, b.page, stepSeconds));
  }
  unlimit(30);
  record("up recovered", await sample(a.page, b.page, stepSeconds));
} catch (error) {
  if (process.env.SHOTS)
    for (const [index, page] of pages.entries())
      await page
        .screenshot({ path: `${process.env.SHOTS}/failure-${index}.png` })
        .catch(() => {});
  throw error;
} finally {
  shapeStop();
  await browser.close();
}
