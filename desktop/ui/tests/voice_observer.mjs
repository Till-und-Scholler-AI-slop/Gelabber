// People in a voice channel for tests/voice_live.rs, like
// web/scripts/parity/call.mjs, whose pages the test also reads and clicks.
// Each person is a headless Chromium with fake microphone and camera.
//   node voice_observer.mjs <channel> <person[:muted|:camera],…>
// Prints "ready" once everyone is in, then answers one JSON command per
// stdin line with one line "= <json>":
//   {"op":"state","who":"jonas"}   the room as that person's page shows it
//   {"op":"click","who":"jonas","name":"Live gehen"}   a button by name
// Ends (everyone leaves) on stdin's end, SIGTERM or SIGINT.
/* global process, console, document */
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { join } from "node:path";
import { createInterface } from "node:readline";

const here = new URL(".", import.meta.url).pathname;
const web = join(here, "../../../web");
const { chromium } = createRequire(join(web, "package.json"))("playwright");
const fixture = JSON.parse(
  await readFile(join(web, "scripts/parity/fixture.json"), "utf8"),
);
const channelName = process.argv[2] ?? "Sofa";
const people = (process.argv[3] ?? "jonas").split(",").map((entry) => {
  const [name, ...flags] = entry.split(":");
  return { name, muted: flags.includes("muted"), camera: flags.includes("camera") };
});
const channel = fixture.servers.home.channels[channelName];
if (!channel) throw new Error(`no channel ${channelName} in the fixture`);

const browser = await chromium.launch({
  executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    "--auto-select-desktop-capture-source=Entire screen",
    "--autoplay-policy=no-user-gesture-required",
  ],
});
let stopping = false;
const stop = async () => {
  if (stopping) return;
  stopping = true;
  await browser.close().catch(() => undefined);
  process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

const pages = {};
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
  const page = await context.newPage();
  await page.goto(`/s/${fixture.servers.home.id}/c/${channel}`, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Beitreten", exact: true }).first().click();
  await page.getByRole("button", { name: /Verlassen|Auflegen/ }).first().waitFor({ timeout: 20000 });
  if (person.muted) await page.getByRole("button", { name: /Stummschalten|Mikrofon aus/ }).first().click();
  if (person.camera) await page.getByRole("button", { name: /Kamera/ }).first().click();
  pages[person.name] = page;
}
console.log("ready");

/** The room on `page`: per card its name, status line and picture. */
function room() {
  const cards = [...document.querySelectorAll(".voice-room-participant")];
  return cards.map((card) => {
    const video = card.querySelector("video");
    const quality = video?.getVideoPlaybackQuality?.();
    return {
      name: card.querySelector(".voice-room-profile-name")?.childNodes[0]?.textContent?.trim() ?? "",
      status: card.querySelector(".voice-room-participant-status")?.textContent?.trim() ?? "",
      speaking: card.classList.contains("is-speaking"),
      video: video
        ? {
            width: video.videoWidth,
            height: video.videoHeight,
            frames: quality?.totalVideoFrames ?? 0,
            time: video.currentTime,
          }
        : null,
    };
  });
}

/** Every stage picture (a camera or a Live) with its label. */
function pictures() {
  return [...document.querySelectorAll("figure.voice-video-tile")].map((figure) => {
    const video = figure.querySelector("video");
    return {
      label: figure.getAttribute("aria-label") ?? figure.querySelector("figcaption")?.textContent ?? "",
      width: video?.videoWidth ?? 0,
      frames: video?.getVideoPlaybackQuality?.().totalVideoFrames ?? 0,
    };
  });
}

const lines = createInterface({ input: process.stdin });
lines.on("close", stop);
for await (const line of lines) {
  let answer;
  try {
    const command = JSON.parse(line);
    const page = pages[command.who];
    if (!page) throw new Error(`nobody called ${command.who}`);
    if (command.op === "state") {
      answer = { room: await page.evaluate(room), pictures: await page.evaluate(pictures) };
    } else if (command.op === "speaking") {
      // `name`'s speaking ring every 20 ms for `ms`: how often it turns.
      answer = await page.evaluate(async ({ name, ms }) => {
        const card = () =>
          [...document.querySelectorAll(".voice-room-participant")].find(
            (c) => c.querySelector(".voice-room-profile-name")?.childNodes[0]?.textContent?.trim() === name,
          );
        let on = 0, samples = 0, flips = 0, last = null;
        const end = performance.now() + ms;
        while (performance.now() < end) {
          const now = card()?.classList.contains("is-speaking") ?? false;
          if (last !== null && last !== now) flips++;
          last = now;
          on += now ? 1 : 0;
          samples++;
          await new Promise((resolve) => setTimeout(resolve, 20));
        }
        return { on, samples, flips };
      }, { name: command.name, ms: command.ms ?? 6000 });
    } else if (command.op === "click") {
      await page.getByRole(command.role ?? "button", { name: command.name, exact: true }).first().click({ timeout: 5000 });
      answer = { ok: true };
    } else {
      throw new Error(`unknown op ${command.op}`);
    }
  } catch (error) {
    answer = { error: String(error) };
  }
  console.log(`= ${JSON.stringify(answer)}`);
}
