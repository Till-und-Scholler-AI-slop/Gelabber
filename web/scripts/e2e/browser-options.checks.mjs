import { test } from "node:test";
import assert from "node:assert/strict";
import { browserLaunchOptions, iceAdapterOptions } from "./browser-options.mjs";

test("loopback ICE is absent by default, requires explicit Firefox relay, and never changes autoplay", () => {
  for (const engine of ["chromium", "firefox"])
    for (const relay of [false, true]) {
      const defaults = browserLaunchOptions(engine, { relay });
      assert.equal(
        defaults.firefoxUserPrefs?.["media.peerconnection.ice.loopback"],
        undefined,
      );
      assert.ok(!JSON.stringify(defaults).includes("autoplay"));
    }
  const adapter = browserLaunchOptions("firefox", {
    relay: true,
    firefoxLoopbackIce: true,
  });
  assert.equal(
    adapter.firefoxUserPrefs["media.peerconnection.ice.loopback"],
    true,
  );
  assert.ok(!JSON.stringify(adapter).includes("autoplay"));
  for (const [engine, relay] of [
    ["firefox", false],
    ["chromium", false],
    ["chromium", true],
  ])
    assert.throws(() =>
      browserLaunchOptions(engine, { relay, firefoxLoopbackIce: true }),
    );
});

test("SFU topology adapter requires Firefox relay and the separately explicit browser adapter", () => {
  for (const options of [
    {},
    { relay: true },
    { relay: true, firefoxLoopbackIce: false },
  ])
    assert.throws(() =>
      browserLaunchOptions("firefox", { ...options, sfuLoopbackIce: true }),
    );
  assert.throws(() =>
    browserLaunchOptions("chromium", {
      relay: true,
      firefoxLoopbackIce: true,
      sfuLoopbackIce: true,
    }),
  );
  const native = browserLaunchOptions("firefox", {
    relay: true,
    firefoxLoopbackIce: true,
    sfuLoopbackIce: true,
  });
  assert.equal(
    native.firefoxUserPrefs["media.peerconnection.ice.loopback"],
    true,
  );
  assert.ok(!JSON.stringify(native).includes("autoplay"));
});

test("adapter env guards reject unknown values and require separate own control env before runtime start", () => {
  for (const env of [
    { GELABBER_E2E_SFU_LOOPBACK_ICE: "false" },
    { GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "yes" },
    { GELABBER_E2E_SFU_LOOPBACK_ICE: "true" },
    {
      GELABBER_E2E_BROWSER: "firefox",
      GELABBER_E2E_NETWORK: "relay",
      GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "true",
      GELABBER_E2E_SFU_LOOPBACK_ICE: "true",
      GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault.env",
    },
  ])
    assert.throws(() => iceAdapterOptions(env));
  const explicit = iceAdapterOptions({
    GELABBER_E2E_BROWSER: "firefox",
    GELABBER_E2E_NETWORK: "relay",
    GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "true",
    GELABBER_E2E_SFU_LOOPBACK_ICE: "true",
    GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault-loopback-control.env",
  });
  assert.equal(explicit.sfuLoopbackIce, true);
  assert.equal(iceAdapterOptions({}).sfuLoopbackIce, false);
});
