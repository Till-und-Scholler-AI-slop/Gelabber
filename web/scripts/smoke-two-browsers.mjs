// Runs against the Compose stack from CI. Each isolated browser context has
// its own account, WebSocket, media peer and cookie jar. Media travels through
// the running SFU and coturn; only camera/screen capture is synthetic.
/* global process, window, navigator, document, setInterval, clearInterval, console, URL */
import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { chromium } from "playwright";

const base = process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1";
const suffix = `${Date.now()}-${process.pid}`;
const password = `Smoke-${suffix}-password`;
const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    "--autoplay-policy=no-user-gesture-required",
  ],
});
const pages = [];

async function participant(name) {
  const context = await browser.newContext({
    permissions: ["camera", "microphone"],
  });
  await context.addInitScript(() => {
    const NativePeer = window.RTCPeerConnection;
    window.__smokePeers = [];
    window.RTCPeerConnection = class extends NativePeer {
      constructor(config) {
        super({ ...config, iceTransportPolicy: "relay" });
        window.__smokePeers.push(this);
      }
    };
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement("canvas");
      canvas.width = 1280;
      canvas.height = 720;
      const ctx = canvas.getContext("2d");
      let frame = 0;
      const timer = setInterval(() => {
        ctx.fillStyle = frame++ % 2 ? "#266ef1" : "#cf4f62";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
      }, 100);
      const stream = canvas.captureStream(15);
      stream
        .getVideoTracks()[0]
        .addEventListener("ended", () => clearInterval(timer));
      return stream;
    };
  });
  const page = await context.newPage();
  pages.push(page);
  await page.goto(`${base}/register`);
  await page.getByLabel("Name").fill(name);
  await page
    .getByLabel("E-Mail-Adresse")
    .fill(`smoke-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${suffix}@example.test`);
  await page.getByLabel("Passwort").fill(password);
  await page.getByRole("button", { name: "Registrieren" }).click();
  await page.waitForURL((url) => !url.pathname.includes("register"));
  return { context, page };
}

async function remoteVideo(page, name) {
  await page.waitForFunction(
    (label) =>
      [...document.querySelectorAll("figure")].some((figure) => {
        const caption = figure.querySelector("figcaption")?.textContent ?? "";
        const video = figure.querySelector("video");
        return (
          caption.includes(label) &&
          video?.srcObject
            ?.getVideoTracks()
            .some((track) => track.readyState === "live") &&
          video.videoWidth > 0
        );
      }),
    name,
    { timeout: 45_000 },
  );
}

async function relaySelected(page) {
  await page.waitForFunction(
    async () => {
      for (const peer of window.__smokePeers ?? []) {
        const report = await peer.getStats();
        const transport = [...report.values()].find(
          (stat) => stat.type === "transport" && stat.selectedCandidatePairId,
        );
        const pair = transport && report.get(transport.selectedCandidatePairId);
        if (pair?.state !== "succeeded") continue;
        if (report.get(pair.localCandidateId)?.candidateType === "relay")
          return true;
      }
      return false;
    },
    null,
    { timeout: 30_000 },
  );
}

try {
  const a = await participant("Smoke A");
  await a.page.getByRole("button", { name: "Server erstellen" }).first().click();
  const serverDialog = a.page.getByRole("dialog", { name: "Server erstellen" });
  await serverDialog.getByLabel("Name").fill(`Smoke ${suffix}`);
  await serverDialog.getByRole("button", { name: "Erstellen" }).click();
  await a.page.waitForURL(/\/s\/[^/]+\/c\//);
  const textUrl = a.page.url();
  await a.page.getByRole("button", { name: "Kanal erstellen" }).click();
  const channelDialog = a.page.getByRole("dialog", { name: "Kanal erstellen" });
  await channelDialog.getByRole("radio", { name: /Voice/ }).check();
  await channelDialog.getByLabel("Name").fill("Smoke Voice");
  await channelDialog.getByRole("button", { name: "Erstellen" }).click();
  await a.page.getByRole("link", { name: "Smoke Voice" }).click();
  const voiceUrl = a.page.url();

  await a.page.getByRole("button", { name: "Leute einladen" }).click();
  const inviteDialog = a.page.getByRole("dialog", { name: /Einladen zu/ });
  await inviteDialog.getByRole("button", { name: "Link erstellen" }).click();
  const invite = await inviteDialog
    .getByRole("textbox", { name: "Einladungslink" })
    .inputValue();
  await inviteDialog.getByRole("button", { name: "Fertig" }).click();

  const b = await participant("Smoke B");
  await b.page.goto(invite);
  await b.page.getByRole("button", { name: "Beitreten" }).click();
  await b.page.waitForURL(/\/s\//);
  await b.page.goto(voiceUrl);

  await a.page.getByRole("button", { name: "Beitreten" }).click();
  await b.page.getByRole("button", { name: "Beitreten" }).click();
  await a.page
    .getByRole("button", { name: "Voice-Einstellungen" })
    .first()
    .click();
  const settings = a.page.getByRole("dialog", { name: "Voice & Video" });
  await settings.locator('input[name="camera-stream-profile"]').first().check();
  await settings.locator('input[name="screen-stream-profile"]').last().check();
  await settings.getByRole("button", { name: "Fertig" }).click();

  await a.page.getByRole("button", { name: "Kamera an" }).first().click();
  await remoteVideo(b.page, "Smoke A");
  await relaySelected(a.page);
  await relaySelected(b.page);

  await a.page
    .getByRole("button", { name: "Bildschirm teilen" })
    .first()
    .click();
  await remoteVideo(b.page, "Bildschirm");
  await a.page.waitForFunction(
    () => {
      const video =
        (window.__smokePeers ?? [])
          .at(-1)
          ?.getSenders()
          .filter((sender) => sender.track?.kind === "video") ?? [];
      if (video.length !== 2) return false;
      const limits = video.map((sender) =>
        sender
          .getParameters()
          .encodings.reduce(
            (sum, encoding) => sum + (encoding.maxBitrate ?? 0),
            0,
          ),
      );
      return (
        limits.every((limit) => limit > 0) &&
        limits.reduce((a, b) => a + b, 0) <= 4_000_000
      );
    },
    null,
    { timeout: 20_000 },
  );
  assert.ok(
    (await a.page
      .locator("details")
      .filter({ hasText: "Sprachdiagnose" })
      .count()) > 0,
  );
  console.log(
    "Two accounts, camera + synthetic screen, SFU video, relay ICE and shared sender budget passed.",
  );

  await a.page.goto(textUrl);
  await b.page.goto(textUrl);
  for (const [extension, type] of [
    ["jpg", "image/jpeg"],
    ["png", "image/png"],
    ["webp", "image/webp"],
  ]) {
    const filename = `smoke.${extension}`;
    const bytes = await readFile(
      new URL(`./fixtures/${filename}`, import.meta.url),
    );
    await a.page.locator('input[type="file"]').setInputFiles({
      name: filename,
      mimeType: type,
      buffer: bytes,
    });
    await a.page
      .locator("form textarea")
      .fill(`Browser upload ${extension} ${suffix}`);
    await a.page.getByRole("button", { name: "Senden" }).click();
    await b.page.waitForFunction(
      (name) =>
        [...document.images].some(
          (image) =>
            image.alt === name && image.complete && image.naturalWidth === 8,
        ),
      filename,
      { timeout: 30_000 },
    );
    const src = await b.page
      .locator(`img[alt="${filename}"]`)
      .getAttribute("src");
    const response = await b.page.request.get(new URL(src, base).href);
    assert.equal(response.status(), 200);
    assert.deepEqual(await response.body(), bytes);
  }
  console.log(
    "JPEG, PNG and WebP upload, second-account rendering and byte-exact download passed.",
  );
} catch (error) {
  await mkdir("/tmp/gelabber-smoke", { recursive: true });
  await Promise.allSettled(
    pages.map((page, index) =>
      page.screenshot({
        path: `/tmp/gelabber-smoke/browser-${index + 1}.png`,
        fullPage: true,
      }),
    ),
  );
  console.error(error);
  process.exitCode = 1;
} finally {
  await browser.close();
}
