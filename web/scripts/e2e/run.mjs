/* global process */
import { startHarness } from "./harness.mjs";
import { mediaScenarios } from "./media.mjs";
import { profiles } from "./profiles.mjs";
import { activeFaultRuntime } from "./fault-runtime.mjs";
if (process.env.GELABBER_E2E_PROFILE) {
  const profile = profiles[process.env.GELABBER_E2E_PROFILE];
  if (
    !profile ||
    process.env.GELABBER_E2E_CASES ||
    process.env.GELABBER_E2E_SUITE
  )
    throw new Error("unknown or conflicting E2E profile");
  process.env.GELABBER_E2E_SUITE = profile.suite;
  process.env.GELABBER_E2E_CASES = profile.cases.join(",");
}
const suite = process.env.GELABBER_E2E_SUITE ?? "all";
if (!["all", "media", "core", "access"].includes(suite))
  throw new Error("unknown E2E suite");
const h = await startHarness();
h.faultRuntime = activeFaultRuntime;
h.report.profile = process.env.GELABBER_E2E_PROFILE ?? null;
if (activeFaultRuntime)
  h.report.ownedFaultRuntime = {
    api: activeFaultRuntime.manifest.sourceSha,
    binarySha256: activeFaultRuntime.manifest.sha256,
    dedicatedDatabaseAndBucket: true,
    replayWindow: 8,
    fixtureRateLimitsDisabled: true,
    media: activeFaultRuntime.media
      ? {
          sourceSha: activeFaultRuntime.media.manifest.sourceSha,
          binarySha256: activeFaultRuntime.media.manifest.sha256,
          authorityViaOwnedRedisProxy: true,
          iceAdapter: activeFaultRuntime.media.iceAdapter,
        }
      : null,
  };
try {
  let f;
  const setup = await h.setup("isolated-app-fixture", [], async () => {
    f = await h.fixture();
    return {
      accounts: 3,
      servers: 1,
      channels: 3,
      registration: "UI",
      invitation: "UI",
      services: "real API/Gateway; SFU joined only in media/access scenarios",
    };
  });
  if (setup.status === "PASS") {
    if (["all", "media"].includes(suite)) await mediaScenarios(h, f);
    if (["all", "core"].includes(suite)) {
      const { coreScenarios } = await import("./core.mjs");
      await coreScenarios(h, f);
    }
    if (["all", "access"].includes(suite)) {
      const { accessScenarios } = await import("./access.mjs");
      await accessScenarios(h, f);
    }
  } else
    h.blocked("requested-scenarios", "fixture-setup-failed", [], {
      setup: true,
    });
} catch (error) {
  await h.setup("suite-interrupted", [], async () => {
    throw error;
  });
} finally {
  await h.finish();
}
