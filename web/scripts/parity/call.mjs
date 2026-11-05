// Puts people from the parity fixture into a voice channel and keeps them
// there until stopped, so both clients can be compared on a running call.
// Each person is a headless Chromium with fake microphone and camera.
// Run in web/: node scripts/parity/call.mjs [channel] [person[:muted|:camera],…]
//   default: Sofa jonas,mira:muted
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
/* global process, console */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "playwright";

const here = new URL(".", import.meta.url).pathname;
const fixture = JSON.parse(await readFile(join(here, "fixture.json"), "utf8"));
const channelName = process.argv[2] ?? "Sofa";
const people = (process.argv[3] ?? "jonas,mira:muted").split(",").map((entry) => {
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
    "--autoplay-policy=no-user-gesture-required",
  ],
});

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
  console.log(`${person.name} is in ${channelName}${person.muted ? " (muted)" : ""}${person.camera ? " (camera)" : ""}`);
}
console.log("in the call; Ctrl+C or SIGTERM ends it");
const stop = async () => {
  await browser.close();
  process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
await new Promise(() => {});
