import assert from "node:assert/strict";

export function browserLaunchOptions(
  engine,
  { relay = false, firefoxLoopbackIce = false, sfuLoopbackIce = false } = {},
) {
  assert.ok(["chromium", "firefox"].includes(engine));
  assert.ok(
    !firefoxLoopbackIce || (engine === "firefox" && relay),
    "Loopback ICE adapter requires local Firefox forced relay",
  );
  assert.ok(
    !sfuLoopbackIce || (engine === "firefox" && relay && firefoxLoopbackIce),
    "SFU loopback adapter requires explicit local Firefox relay and browser loopback adapter",
  );
  return engine === "chromium"
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
          ...(firefoxLoopbackIce
            ? { "media.peerconnection.ice.loopback": true }
            : {}),
        },
      };
}

export function iceAdapterOptions(env) {
  const engine = env.GELABBER_E2E_BROWSER ?? "chromium";
  const relay = env.GELABBER_E2E_NETWORK === "relay";
  for (const key of [
    "GELABBER_E2E_FIREFOX_LOOPBACK_ICE",
    "GELABBER_E2E_SFU_LOOPBACK_ICE",
  ])
    assert.ok(
      env[key] === undefined || env[key] === "" || env[key] === "true",
      "Unknown local ICE adapter value",
    );
  const options = {
    engine,
    relay,
    firefoxLoopbackIce: env.GELABBER_E2E_FIREFOX_LOOPBACK_ICE === "true",
    sfuLoopbackIce: env.GELABBER_E2E_SFU_LOOPBACK_ICE === "true",
  };
  browserLaunchOptions(engine, options);
  if (options.sfuLoopbackIce)
    assert.ok(
      env.GELABBER_E2E_MEDIA_FAULT_ENV?.endsWith(
        "/media-fault-loopback-control.env",
      ),
      "Own separate SFU loopback control env required",
    );
  return options;
}
