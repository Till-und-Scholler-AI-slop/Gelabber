/* global process, URL */
// Chromium for the call scenario: one browser per label, so sampler.py can
// measure its whole process tree; fake camera and microphone.
import { chromium } from "playwright";
import { labelProcess } from "./phases.mjs";

export async function launch(name, extraArgs = []) {
  const server = await chromium.launchServer({
    // The full Chromium in its new headless mode, not the headless shell:
    // its process layout and memory are those of a desktop Chrome.
    ...(process.env.GELABBER_LOAD_BROWSER_EXECUTABLE
      ? { executablePath: process.env.GELABBER_LOAD_BROWSER_EXECUTABLE }
      : { channel: "chromium" }),
    headless: true,
    args: [
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      "--autoplay-policy=no-user-gesture-required",
      ...extraArgs,
    ],
  });
  const pid = server.process().pid;
  labelProcess(pid, name);
  const browser = await chromium.connect(server.wsEndpoint());
  return {
    browser,
    async close() {
      await browser.close().catch(() => {});
      await server.close().catch(() => {});
      labelProcess(pid, null);
    },
  };
}

/** A context signed in as `client` (a client.mjs Client of the same origin). */
export async function signedInPage(
  browser,
  client,
  path,
  viewport = { width: 1280, height: 800 },
) {
  const context = await browser.newContext({
    viewport,
    permissions: ["microphone", "camera"],
  });
  const { hostname } = new URL(client.base);
  await context.addCookies(
    [...client.jar].map(([name, value]) => ({
      name,
      value,
      domain: hostname,
      path: "/",
      sameSite: "Lax",
    })),
  );
  const page = await context.newPage();
  await page.goto(client.base + path);
  return { context, page };
}
