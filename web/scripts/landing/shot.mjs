// Real screenshots of the web client from the server seed.mjs made:
//   en, de   the landing page's picture, public/images/landing/app-<lang>.png:
//            Lena in general while Jonas is live in Sofa, 1280×800 at
//            device scale 1, as hosted-landing.webp shows it.
//   readme   the README's pictures from the English server,
//            docs/assets/readme-{hero,chat,voice}.png: the overview, general,
//            and Sofa with Lena in the call watching Jonas' Live, 1280×800 at
//            device scale 1.5.
// Lena's account language is set to the shot's through the language setting
// (PUT /api/me/locale). Calls come from scripts/parity/call.mjs with this
// fixture. Reduced motion, dark scheme, no caret, the pointer out of the way.
// Run in web/ after seed.mjs, with the same stack running:
//   node scripts/landing/shot.mjs [en|de|readme] [out-dir]
// GELABBER_PARITY_BROWSER names the Chromium binary (default /usr/bin/chromium).
/* global process, console, URL, setTimeout, document, window */
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "playwright";

const MODE = process.argv[2] ?? "en";
if (!["en", "de", "readme"].includes(MODE))
  throw new Error(`en, de or readme, not ${MODE}`);
const here = new URL(".", import.meta.url).pathname;
const web = join(here, "../..");
const LANG = MODE === "readme" ? "en" : MODE;
const OUT =
  process.argv[3] ??
  join(web, MODE === "readme" ? "../docs/assets" : "public/images/landing");
const FIXTURE = join(here, `fixture-${LANG}.json`);
const fixture = JSON.parse(await readFile(FIXTURE, "utf8"));
const { home } = fixture.servers;
const SIZE = { width: 1280, height: 800 };
const TEXT = {
  en: { join: "Join", watch: "Watch", leave: /Leave|Hang up/ },
  de: { join: "Beitreten", watch: "Zuschauen", leave: /Verlassen|Auflegen/ },
}[LANG];

/** call.mjs with this fixture, resolved once everyone is in. */
function startCall(people) {
  const child = spawn(
    process.execPath,
    [join(web, "scripts/parity/call.mjs"), "Sofa", people],
    {
      stdio: ["ignore", "pipe", "inherit"],
      env: { ...process.env, GELABBER_PARITY_FIXTURE: FIXTURE },
    },
  );
  return new Promise((resolve, reject) => {
    let out = "";
    child.stdout.on("data", (chunk) => {
      out += chunk;
      if (out.includes("in the call")) setTimeout(() => resolve(child), 2000);
    });
    child.on("exit", (code) =>
      reject(new Error(`call.mjs exited with ${code}`)),
    );
  });
}

const browser = await chromium.launch({
  executablePath: process.env.GELABBER_PARITY_BROWSER ?? "/usr/bin/chromium",
  args: [
    "--font-render-hinting=none",
    "--disable-lcd-text",
    // Lena joins the README's call: fake devices, no prompt.
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
  ],
});
const call = await startCall(
  MODE === "readme" ? "jonas:live,mira:muted,tom:camera" : "jonas:live",
);
const stop = () => call.kill("SIGTERM");
process.on("exit", stop);

const context = await browser.newContext({
  baseURL: fixture.base,
  viewport: SIZE,
  deviceScaleFactor: MODE === "readme" ? 1.5 : 1,
  reducedMotion: "reduce",
  colorScheme: "dark",
  locale: LANG === "en" ? "en-GB" : "de-DE",
  timezoneId: "Europe/Berlin",
  permissions: ["microphone", "camera"],
});
context.setDefaultTimeout(20_000);
const session = await (await context.request.get("/api/auth/session")).json();
const login = await context.request.post("/api/auth/login", {
  data: { email: fixture.users.lena.email, password: fixture.password },
  headers: { "X-CSRF-Token": session.csrf_token },
});
if (!login.ok()) throw new Error(`login lena: ${login.status()}`);
const { csrf_token } = await login.json();
const locale = await context.request.put("/api/me/locale", {
  data: { locale: LANG },
  headers: { "X-CSRF-Token": csrf_token },
});
if (!locale.ok()) throw new Error(`locale lena: ${locale.status()}`);
// The desktop-app toast would cover the corner (desktopApp.ts).
await context.addInitScript(() => {
  try {
    window.localStorage.setItem("gelabber.desktopApp.offered", "1");
  } catch {
    // No storage on about:blank.
  }
});
const page = await context.newPage();

async function shoot(path, file, before) {
  await page.goto(path, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  if (before) await before();
  await page.addStyleTag({ content: "*{caret-color:transparent !important}" });
  await page.mouse.move(SIZE.width - 1, SIZE.height - 1);
  // Let lists settle, late images decode and hover fades end.
  await page.waitForTimeout(1200);
  await page.screenshot({ path: join(OUT, file) });
  console.log(`${file}: ${page.url()}`);
}

const general = `/s/${home.id}/c/${home.channels.general}`;
// The channel header's notice that Jonas is live, as in the preview.
const liveNotice = () =>
  page.getByRole("button", { name: TEXT.watch, exact: true }).first().waitFor();

if (MODE === "readme") {
  await shoot(`/s/${home.id}`, "readme-hero.png");
  await shoot(general, "readme-chat.png", liveNotice);
  await shoot(
    `/s/${home.id}/c/${home.channels.Sofa}`,
    "readme-voice.png",
    async () => {
      await page
        .getByRole("button", { name: TEXT.join, exact: true })
        .first()
        .click();
      await page.getByRole("button", { name: TEXT.leave }).first().waitFor();
      await page
        .getByRole("button", { name: TEXT.watch, exact: true })
        .first()
        .click();
      await page.waitForTimeout(3000);
    },
  );
} else {
  await shoot(general, `app-${LANG}.png`, liveNotice);
}
await context.close();
await browser.close();
stop();
