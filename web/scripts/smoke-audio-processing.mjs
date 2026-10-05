// Local native AudioWorklet/recording acceptance. Start Vite on 5179 first.
/* global process, console, window, document, URL, navigator */
import assert from "node:assert/strict";
import { chromium, firefox } from "playwright";
const url = new URL(process.env.GELABBER_AUDIO_URL ?? "http://127.0.0.1:5179");
assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(url.hostname));
const results = [];
for (const engine of [chromium, firefox]) {
  const browser = await engine.launch(
    engine === chromium
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
  try {
    const page = await browser.newPage({
      viewport: { width: 390, height: 844 },
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/audio-processing-smoke", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: `<!doctype html><html><meta name="viewport" content="width=device-width, initial-scale=1"><body style="padding:16px"><div id="settings"></div><button id="probe">DSP probe</button><button id="play">Play comparison</button><script type="module">
      import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>(type)=>type;window.__vite_plugin_react_preamble_installed__=true;
      await import('/src/index.css');
      const {default:React}=await import('/node_modules/.vite/deps/react.js');const {default:{createRoot}}=await import('/node_modules/.vite/deps/react-dom_client.js');
      const formSource=await fetch('/src/voice/VoiceSettings.tsx').then(response=>response.text());
      // Match Vite's current HMR-qualified dependencies so the form and probe share one store.
      const dependency=(name)=>import(formSource.match(new RegExp('from "(/src/voice/'+name+'\\\\.ts[^"]*)"'))[1]);
      const {MediaSettingsForm}=await import('/src/voice/VoiceSettings.tsx');const settings=await dependency('settings');const processing=await dependency('audioProcessing');
      window.settings=settings;window.processing=processing;
      createRoot(document.querySelector('#settings')).render(React.createElement(MediaSettingsForm,{section:'audio'}));
      document.querySelector('#probe').onclick=async()=>{try{const result=await processing.captureMicrophone(c=>navigator.mediaDevices.getUserMedia(c), settings.useMediaSettings.getState());window.captured=result;window.audioProbe=result.processor.info;}catch(e){window.audioProbe={error:e.message};}};
      document.querySelector('#play').onclick=()=>document.querySelector('audio').play().then(()=>window.played=true);
      window.ready=true;
    </script></body></html>`,
      }),
    );
    await page.goto(url.origin);
    await page.waitForFunction(
      () => document.querySelector("#root")?.childElementCount > 0,
    );
    await page.goto(`${url.origin}/audio-processing-smoke`);
    await page.waitForFunction(() => window.ready);
    await page.evaluate(() => {
      const capture = navigator.mediaDevices.getUserMedia.bind(
        navigator.mediaDevices,
      );
      window.audioCaptures = [];
      navigator.mediaDevices.getUserMedia = async (constraints) => {
        const stream = await capture(constraints);
        window.audioCaptures.push(stream);
        return stream;
      };
      const revoke = URL.revokeObjectURL.bind(URL);
      window.revokedClips = [];
      URL.revokeObjectURL = (clip) => {
        window.revokedClips.push(clip);
        revoke(clip);
      };
    });
    // Legacy phone economy remains accurately displayed instead of claiming 64 kbit/s.
    await page.evaluate(() =>
      window.settings.useMediaSettings
        .getState()
        .patch({ economyMode: true, quality: "phone" }),
    );
    await page.getByText(/Sprache 24 kbit\/s/).waitFor();
    await page.evaluate(() =>
      window.settings.useMediaSettings
        .getState()
        .patch({ economyMode: false, videoUploadLimit: 12_000_000 }),
    );
    await page.getByText(/Audio ohne Bitratengrenze.*12 Mbit\/s/).waitFor();
    await page.evaluate(() =>
      window.settings.useMediaSettings
        .getState()
        .patch({ videoUploadLimit: 0 }),
    );
    await page
      .getByText(/Keine Bitratengrenze durch Gelabber\. Browser/)
      .waitFor();
    await page.click("#probe");
    await page.waitForFunction(() => window.audioProbe);
    const enhanced = await page.evaluate(() => window.audioProbe);
    assert.equal(enhanced.actual, "enhanced", JSON.stringify(enhanced));
    assert.equal(enhanced.sampleRate, 48000);
    assert.equal(enhanced.channels, 1);
    await page.evaluate(() => {
      window.captured.processor.dispose();
      window.captured.raw.getTracks().forEach((t) => t.stop());
      window.audioProbe = null;
      window.settings.useMediaSettings
        .getState()
        .patch({ processingMode: "original", echoCancellation: false });
    });
    await page.click("#probe");
    await page.waitForFunction(() => window.audioProbe);
    const original = await page.evaluate(() => window.audioProbe);
    assert.equal(original.actual, "original");
    assert.equal(original.noiseSuppression, false);
    assert.equal(original.autoGainControl, false);
    await page.evaluate(() => {
      window.captured.processor.dispose();
      window.captured.raw.getTracks().forEach((t) => t.stop());
      window.settings.useMediaSettings
        .getState()
        .patch({ processingMode: "enhanced" });
    });
    await page
      .getByRole("button", { name: "Mikrofon testen und vergleichen" })
      .click();
    await page
      .getByRole("button", { name: "Test starten", exact: true })
      .click();
    await page.waitForFunction(() =>
      document.body.textContent.includes("Lokale Rauschunterdrückung aktiv"),
    );
    await page.getByRole("button", { name: "A/B-Aufnahme starten" }).click();
    await page.waitForTimeout(1200);
    await page
      .getByRole("button", { name: "Aufnahme beenden", exact: true })
      .click();
    await page.waitForFunction(
      () => document.querySelectorAll('audio[src^="blob:"]').length === 2,
    );
    await page.evaluate(() => {
      const button = document.createElement("button");
      button.id = "local-play";
      button.textContent = "Play local clip";
      button.onclick = () =>
        document
          .querySelector("audio")
          .play()
          .then(() => (window.played = true));
      document.querySelector("dialog").append(button);
    });
    await page.click("#local-play");
    await page.waitForFunction(() => window.played);
    await page.waitForFunction(
      () => document.querySelector("audio").currentTime > 0.1,
    );
    await page.getByRole("button", { name: "Aufnahmen löschen" }).click();
    assert.equal(await page.locator("audio").count(), 0);
    await page.getByRole("button", { name: "Schließen", exact: true }).click();
    assert.equal(
      await page.evaluate(() =>
        window.audioCaptures.every((stream) =>
          stream.getTracks().every((track) => track.readyState === "ended"),
        ),
      ),
      true,
    );
    assert.equal(
      await page.evaluate(() => window.revokedClips.length >= 2),
      true,
    );
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
      true,
    );
    assert.deepEqual(errors, []);
    // Fresh document: corrupted/missing assets must visibly recapture browser filters.
    await page.reload();
    await page.waitForFunction(() => window.ready);
    await page.route("**/audio/rnnoise.wasm*", (route) =>
      route.fulfill({ status: 404, body: "missing" }),
    );
    await page.click("#probe");
    await page.waitForFunction(() => window.audioProbe);
    const fallback = await page.evaluate(() => window.audioProbe);
    assert.equal(fallback.actual, "browser");
    assert.match(fallback.message, /Browser(filter|-Modus)/);
    await page.evaluate(() => {
      window.captured.processor.dispose();
      window.captured.raw.getTracks().forEach((t) => t.stop());
    });
    results.push({
      browser: engine.name(),
      enhanced,
      original,
      fallback,
      localRecordingPlayback: true,
      mobileWidth: 390,
      physicalDevice: false,
    });
  } finally {
    await browser.close();
  }
}
console.log(JSON.stringify(results, null, 2));
