// Local-only native playback/setting acceptance. Run Vite first (npm run dev).
// No autoplay override, microphone capture, accounts or production writes.
/* global process, window, HTMLMediaElement, console, URL */
import assert from "node:assert/strict";
import { chromium, firefox } from "playwright";
const url = new URL(
  process.env.GELABBER_CALL_SOUNDS_URL ?? "http://127.0.0.1:5173",
);
assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(url.hostname));
assert.ok(!url.username && !url.password && !url.search && !url.hash);
const results = [];
for (const engine of [chromium, firefox]) {
  const browser = await engine.launch(
    engine === chromium && process.env.BRAVE_PATH
      ? { executablePath: process.env.BRAVE_PATH }
      : {},
  );
  try {
    const page = await browser.newPage({
      viewport: { width: 390, height: 844 },
    });
    page.on("pageerror", (error) => console.error(error.message));
    await page.route("**/call-sounds-smoke", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: `<!doctype html><html><meta name="viewport" content="width=device-width, initial-scale=1"><body>
      <button id="join">Join gesture</button><div id="settings"></div>
      <script type="module">
        import * as sounds from '/src/voice/callSounds.ts';
        import { useMediaSettings } from '/src/voice/settings.ts';
        window.sounds = sounds; window.settings = useMediaSettings;
        document.querySelector('#join').onclick = sounds.unlockCallSounds;
        import RefreshRuntime from '/@react-refresh';
        RefreshRuntime.injectIntoGlobalHook(window);
        window.$RefreshReg$ = () => {};
        window.$RefreshSig$ = () => (type) => type;
        window.__vite_plugin_react_preamble_installed__ = true;
        await import('/src/index.css');
        const { default: React } = await import('/node_modules/.vite/deps/react.js');
        const { default: { createRoot } } = await import('/node_modules/.vite/deps/react-dom_client.js');
        const { MediaSettingsForm } = await import('/src/voice/VoiceSettings.tsx');
        createRoot(document.querySelector('#settings')).render(React.createElement(MediaSettingsForm, { section: 'notifications' }));
        document.body.style.padding = '16px';
        window.ready = true;
      </script></body></html>`,
      }),
    );
    await page.addInitScript(() => {
      window.probe = { plays: [], ended: 0, errors: [] };
      const original = HTMLMediaElement.prototype.play;
      HTMLMediaElement.prototype.play = function () {
        const clip = { volume: this.volume, duration: 0, ok: false };
        window.probe.plays.push(clip);
        this.addEventListener(
          "ended",
          () => {
            window.probe.ended++;
            clip.duration = this.duration;
          },
          { once: true },
        );
        return original.call(this).then(
          () => {
            clip.ok = true;
          },
          (error) => {
            window.probe.errors.push(error.name);
            throw error;
          },
        );
      };
    });
    await page.goto(`${url.origin}/call-sounds-smoke`);
    await page.waitForFunction(() => window.ready);
    await page.click("#join");
    await page.waitForFunction(() => window.probe.ended === 1);
    // Outside the click: models the later gateway join acknowledgement.
    await page.evaluate(() => window.sounds.playCallSound("join"));
    await page.waitForFunction(() => window.probe.ended === 2);
    let probe = await page.evaluate(() => window.probe);
    assert.equal(probe.plays[1].ok, true);
    assert.equal(probe.plays[1].volume, 0.35);
    assert.ok(Math.abs(probe.plays[1].duration - 0.24) < 0.01);
    await page.evaluate(() => {
      window.sounds.setCallSoundsDeafened(true);
      window.sounds.playCallSound("join");
      window.sounds.playCallSound("leave");
    });
    assert.equal(await page.evaluate(() => window.probe.plays.length), 2);
    await page.evaluate(() => window.sounds.playCallSound("deafen"));
    await page.waitForFunction(() => window.probe.ended === 3);
    await page.evaluate(() => {
      window.settings.getState().patch({ callSounds: false });
      window.sounds.playCallSound("unmute");
    });
    assert.equal(await page.evaluate(() => window.probe.plays.length), 3);
    await page.reload();
    await page.waitForFunction(() => window.ready);
    assert.equal(
      await page.evaluate(() => window.settings.getState().callSounds),
      false,
    );
    await page.evaluate(() => {
      window.settings
        .getState()
        .patch({ callSounds: true, callSoundVolume: 0.2, outputVolume: 0.5 });
    });
    await page.click("#join");
    await page.waitForFunction(() => window.probe.ended === 1);
    await page.evaluate(() => window.sounds.playCallSound("leave"));
    await page.waitForFunction(() => window.probe.ended === 2);
    probe = await page.evaluate(() => window.probe);
    assert.equal(probe.plays[1].volume, 0.1);
    assert.deepEqual(probe.errors, []);
    await page.getByRole("button", { name: "Testton abspielen" }).click();
    await page.waitForFunction(() => window.probe.ended === 3);
    await page.getByLabel("Signaltöne im Call", { exact: true }).uncheck();
    assert.equal(
      await page
        .getByRole("button", { name: "Testton abspielen" })
        .isDisabled(),
      true,
    );
    if (process.env.GELABBER_CALL_SOUNDS_SCREENSHOT)
      await page.screenshot({
        path: `${process.env.GELABBER_CALL_SOUNDS_SCREENSHOT}-${engine.name()}.png`,
        fullPage: true,
      });
    results.push({
      engine: engine.name(),
      version: browser.version(),
      status: "passed",
      checks: [
        "native delayed playback",
        "decoded PCM duration",
        "deafen",
        "explicit control feedback",
        "disable",
        "persistence",
        "combined volume",
        "mobile settings and preview",
      ],
    });
  } finally {
    await browser.close();
  }
}
console.log(JSON.stringify(results, null, 2));
