/* global process */
import { startHarness } from "./harness.mjs";
import { mediaScenarios } from "./media.mjs";
const suite = process.env.GELABBER_E2E_SUITE ?? "all";
if (!["all", "media", "core", "access"].includes(suite))
  throw new Error("unknown E2E suite");
const h = await startHarness();
try {
  let f;
  const setup = await h.run("isolated-app-fixture", [], async () => {
    f = await h.fixture();
    return {
      accounts: 3,
      servers: 1,
      channels: 3,
      registration: "UI",
      invitation: "UI",
      services: "real API/Gateway/SFU",
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
  } else h.blocked("requested-scenarios", "fixture-setup-failed");
} catch (error) {
  await h.run("suite-interrupted", [], async () => {
    throw error;
  });
} finally {
  await h.finish();
}
