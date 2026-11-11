// Puts people from the parity fixture into a voice channel and keeps them
// there until stopped, so both clients can be compared on a running call.
// Each person is a headless Chromium with fake microphone and camera.
// Run in web/: node scripts/parity/call.mjs [channel] [person[:muted|:camera|:live|:watch],…]
//   default: Sofa jonas,mira:muted
// `watch` presses every "Zuschauen" in the room, and keeps checking once a
// second, so a Live that starts later is watched too.
// Cameras show one still picture (a generated y4m) and a Live a still
// canvas instead of Chromium's moving fake screen, so web and native shots
// get the same frames.
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
// GELABBER_PARITY_FIXTURE names another fixture in the same shape (default
// fixture.json here), such as the landing screenshot's (scripts/landing).
/* global process, console, URL, Buffer, navigator, document, setInterval */
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(
  await readFile(process.env.GELABBER_PARITY_FIXTURE ?? join(here, "fixture.json"), "utf8"),
);
const channelName = process.argv[2] ?? "Sofa";
const people = (process.argv[3] ?? "jonas,mira:muted").split(",").map((entry) => {
  const [name, ...flags] = entry.split(":");
  return {
    name,
    muted: flags.includes("muted"),
    camera: flags.includes("camera"),
    live: flags.includes("live"),
    watch: flags.includes("watch"),
  };
});
const channel = fixture.servers.home.channels[channelName];
if (!channel) throw new Error(`no channel ${channelName} in the fixture`);

/** A 640×360 still in I420: soft colour fields that survive the encoder. */
function stillY4m() {
  const [w, h] = [640, 360];
  const y = Buffer.alloc(w * h);
  const u = Buffer.alloc((w / 2) * (h / 2));
  const v = Buffer.alloc((w / 2) * (h / 2));
  for (let row = 0; row < h; row++)
    for (let col = 0; col < w; col++) {
      const box = col > 220 && col < 420 && row > 100 && row < 260;
      y[row * w + col] = box ? 200 : 60 + Math.round((col / w) * 90);
    }
  for (let row = 0; row < h / 2; row++)
    for (let col = 0; col < w / 2; col++) {
      u[row * (w / 2) + col] = 150 - Math.round((row / (h / 2)) * 40);
      v[row * (w / 2) + col] = 100 + Math.round((col / (w / 2)) * 50);
    }
  const head = Buffer.from(`YUV4MPEG2 W${w} H${h} F30:1 Ip A1:1 C420jpeg\nFRAME\n`);
  return Buffer.concat([head, y, u, v]);
}
const scratch = await mkdtemp(join(tmpdir(), "gelabber-call-"));
const still = join(scratch, "camera.y4m");
await writeFile(still, stillY4m());

const browser = await chromium.launch({
  executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-video-capture=${still}`,
    "--auto-select-desktop-capture-source=Entire screen",
    "--autoplay-policy=no-user-gesture-required",
  ],
});

/** Presses each "Zuschauen" on the page now and once a second from then on. */
function watchEveryLive(page) {
  let busy = false;
  const press = async () => {
    if (busy) return;
    busy = true;
    try {
      const buttons = page.getByRole("button", { name: "Zuschauen", exact: true });
      // Each press removes its button, so the first one is always next.
      for (let n = await buttons.count(); n > 0; n--) await buttons.first().click({ timeout: 2000 });
    } catch {
      // A Live that ended under the click: the next round looks again.
    } finally {
      busy = false;
    }
  };
  void press();
  setInterval(press, 1000);
}

for (const person of people) {
  const user = fixture.users[person.name];
  const context = await browser.newContext({
    baseURL: fixture.base,
    viewport: { width: 1280, height: 800 },
    permissions: ["microphone", "camera"],
    locale: "de-DE",
  });
  const session = await (await context.request.get("/api/auth/session")).json();
  const login = await context.request.post("/api/auth/login", {
    data: { email: user.email, password: fixture.password },
    headers: { "X-CSRF-Token": session.csrf_token },
  });
  if (!login.ok()) throw new Error(`login ${person.name}: ${login.status()}`);
  // getDisplayMedia answers with a canvas that keeps one picture.
  await context.addInitScript(() => {
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement("canvas");
      canvas.width = 1280;
      canvas.height = 720;
      const draw = () => {
        const g = canvas.getContext("2d");
        const fill = g.createLinearGradient(0, 0, 1280, 720);
        fill.addColorStop(0, "#1d4a6b");
        fill.addColorStop(1, "#7a3f63");
        g.fillStyle = fill;
        g.fillRect(0, 0, 1280, 720);
        g.fillStyle = "#e8d8a0";
        g.fillRect(420, 220, 440, 280);
      };
      draw();
      // A canvas sends a frame when it is drawn: the same picture again.
      setInterval(draw, 100);
      return canvas.captureStream(30);
    };
  });
  const page = await context.newPage();
  await page.goto(`/s/${fixture.servers.home.id}/c/${channel}`, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Beitreten", exact: true }).first().click();
  await page.getByRole("button", { name: /Verlassen|Auflegen/ }).first().waitFor({ timeout: 20000 });
  if (person.muted) await page.getByRole("button", { name: /Stummschalten|Mikrofon aus/ }).first().click();
  if (person.camera) await page.getByRole("button", { name: /Kamera/ }).first().click();
  if (person.live) {
    await page.getByRole("button", { name: "Live gehen", exact: true }).first().click();
    await page.getByRole("menuitem", { name: "Live gehen", exact: true }).click();
    await page.getByRole("button", { name: "Live gehen", pressed: true }).first().waitFor();
  }
  if (person.watch) watchEveryLive(page);
  const flags = ["muted", "camera", "live", "watch"].filter((flag) => person[flag]);
  console.log(`${person.name} is in ${channelName}${flags.map((f) => ` (${f})`).join("")}`);
}
console.log("in the call; Ctrl+C or SIGTERM ends it");
const stop = async () => {
  await browser.close();
  await rm(scratch, { recursive: true, force: true });
  process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
await new Promise(() => {});
