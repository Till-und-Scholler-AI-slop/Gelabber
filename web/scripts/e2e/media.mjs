import { nativeEvaluate } from "./native-evaluate.mjs";
/* global window, document, innerWidth */
import {
  check,
  click,
  navigate,
  snapshot,
  until,
  observe,
} from "./harness.mjs";

export const activePeers = (s) =>
  s.peers.filter((p) => p.connection !== "closed");
export function armPlaybackRetry() {
  // Keep unmute/autoplay callbacks blocked until the real user click reaches
  // the retry button; lifting the fault earlier can remove that button.
  const release = (event) => {
    if (
      !event.isTrusted ||
      !["Wiedergabe starten", "Ton starten"].includes(
        event.target.closest?.("button")?.textContent?.trim(),
      )
    )
      return;
    window.__e2e.rejectPlayback = false;
    window.removeEventListener("click", release, true);
  };
  window.addEventListener("click", release, true);
}
export const decoded = (s) =>
  activePeers(s)
    .flatMap((p) => p.inbound)
    .filter((r) => r.kind === "video")
    .reduce((n, r) => n + (r.frames ?? 0), 0);
const encoded = (s) =>
  activePeers(s)
    .flatMap((p) => p.outbound)
    .filter((r) => r.kind === "video")
    .reduce((n, r) => n + (r.frames ?? 0), 0);
function sourceMatches(video, color) {
  return (
    video?.pixels &&
    video.pixels.source.every((v, i) => Math.abs(v - color[i]) < 45)
  );
}
/** Render assertions select their source through the real Watch action. */
export async function watchSource(
  actor,
  kind,
  { publisherName, budget = 5_000 } = {},
) {
  if (kind !== "live" && kind !== "screen") return { action: "camera" };
  const suffix = kind === "live" ? "— Live" : "— Bildschirm";
  let tiles = actor.page.locator("figure").filter({ hasText: suffix });
  if (publisherName)
    tiles = tiles.filter({ hasText: `${publisherName} ${suffix}` });
  const deadline = Date.now() + budget;
  await tiles.first().waitFor({ state: "visible", timeout: budget });
  check((await tiles.count()) === 1, "expected-source-tile-is-ambiguous", {
    kind,
    publisherName,
  });
  const tile = tiles.first();
  const start = tile.getByRole("button", { name: "Zuschauen", exact: true });
  const stop = tile.getByRole("button", {
    name: "Nicht mehr zuschauen",
    exact: true,
  });
  const globalStop = actor.page
    .getByRole("region", { name: "Aktive Medien", exact: true })
    .getByRole("button", { name: "Nicht mehr zuschauen", exact: true });
  const choice = await until(
    async () => ({
      start: await start.count(),
      stop: await stop.count(),
      globalStop: await globalStop.count(),
    }),
    (state) =>
      state.start === 1 ||
      state.stop === 1 ||
      (kind === "live" && state.globalStop === 1),
    "expected-source-watch-control-not-mounted",
    Math.max(1, deadline - Date.now()),
  );
  if (choice.start === 1) {
    await start.click({ timeout: Math.max(1, deadline - Date.now()) });
    return { action: "clicked", kind, publisherName };
  }
  return {
    action: choice.stop === 1 ? "already-selected" : "watch-peer",
    kind,
    publisherName,
  };
}
export async function progress(
  actor,
  {
    kind = "live",
    color = [220, 30, 30],
    budget = 5_000,
    relay = false,
    publisherName,
  } = {},
) {
  const start = Date.now();
  const watchAction = await watchSource(actor, kind, { publisherName, budget });
  const first = await until(
    () => snapshot(actor),
    (s) => {
      const video = s.videos.find((v) => v.kind === kind);
      return (
        decoded(s) > 0 &&
        video?.width === 640 &&
        !video.paused &&
        sourceMatches(video, color)
      );
    },
    "decoded-correct-source-first-frame-deadline",
    Math.max(1, budget - (Date.now() - start)),
  );
  const firstFrameMs = Date.now() - start;
  const initial = first.videos.find((v) => v.kind === kind);
  const last = await until(
    () => snapshot(actor),
    (s) => {
      const video = s.videos.find((v) => v.kind === kind);
      return (
        decoded(s) > decoded(first) + 3 &&
        sourceMatches(video, color) &&
        video.renderedFrames > initial.renderedFrames &&
        video.pixels.motion.some(
          (v, i) => Math.abs(v - initial.pixels.motion[i]) > 80,
        )
      );
    },
    "decoded-rendered-changing-frames-deadline",
    5_000,
  );
  const paths = activePeers(last).flatMap((p) => p.selected);
  check(
    paths.length > 0 &&
      paths.every(
        (p) =>
          p.state === "succeeded" &&
          (relay
            ? p.local === "relay"
            : p.local !== "relay" && p.remote !== "relay"),
      ),
    "selected-network-path-mismatch",
    { paths },
  );
  return { watchAction, firstFrameMs, first, last };
}
async function reset(f) {
  for (const actor of [f.owner, f.member, f.watcher])
    await navigate(actor, f.voicePath, f.base);
}
async function begin(f) {
  await reset(f);
  await click(f.owner, "Beitreten");
  await click(f.owner, "Go Live");
  f.liveStartedAt = Date.now();
  await until(
    () => snapshot(f.owner),
    (s) => encoded(s) > 0,
    "publisher-encode-deadline",
  );
  await click(f.watcher, "Zuschauen");
}
export async function interrupt(actor, plane) {
  check(["gateway", "media"].includes(plane), "unsupported-fault-plane");
  const count = await nativeEvaluate(
    actor,
    (plane) => {
      const sockets = window.__e2e.sockets.filter(
        (s) => s.plane === plane && s.ws.readyState === 1,
      );
      for (const s of sockets) s.ws.close(4000, "e2e-local-fault");
      return sockets.length;
    },
    plane,
  );
  check(count > 0, "fault-target-socket-missing");
  return count;
}
export async function mediaScenarios(h, f) {
  const budget = h.relay ? 10_000 : 5_000;
  const options = { relay: h.relay, budget };
  async function initialWatch() {
    await begin(f);
    const frames = await progress(f.watcher, options);
    check(
      frames.last.micCalls === 0 &&
        frames.last.cameraCalls === 0 &&
        frames.last.displayCalls === 0,
      "watch-requested-capture",
      frames.last,
    );
    check(
      activePeers(frames.last).every((p) => p.senders === 0),
      "watch-published-media",
      frames.last,
    );
    return frames;
  }
  let first = await h.run(
    "live-watch-default-autoplay-no-mic",
    ["01", "08b"],
    initialWatch,
  );
  if (first.status === "NOT_RUN" && h.wants("late-watch-after-30s"))
    first = await h.setup(
      "late-watch-live-fixture",
      ["01", "08b"],
      initialWatch,
    );
  if (first.status === "PASS") {
    await h.run("late-watch-after-30s", ["01"], async () => {
      const baseline = await snapshot(f.owner);
      const late = await h.actor("LateWatch");
      await f.join(late);
      // This is the specified late-join age, continuously checking encode/capture liveness.
      const start = f.liveStartedAt;
      while (Date.now() - start < 30_000) {
        await observe(
          Math.min(1_000, 30_000 - (Date.now() - start)),
          async () => {
            const s = await snapshot(f.owner);
            check(
              s.displayCalls === baseline.displayCalls &&
                s.captures.some((c) => c.state === "live"),
              "late-join-publisher-restarted",
            );
            return s;
          },
        );
      }
      await click(late, "Zuschauen");
      const frames = await progress(late, options);
      check(frames.last.micCalls === 0, "late-watch-requested-mic");
      const after = await snapshot(f.owner);
      check(
        after.displayCalls === baseline.displayCalls &&
          encoded(after) > encoded(baseline),
        "late-join-publisher-progress",
        after,
      );
      await late.context.close();
      return { ageMs: Date.now() - start, ...frames, publisher: after };
    });
  } else h.blocked("late-watch-after-30s", "initial-live-watch-failed", ["01"]);
  for (const [id, plane] of [
    ["media-ws-capture-preserving-recovery", "media"],
    ["gateway-only-preserves-peer", "gateway"],
  ]) {
    await h.run(id, ["02", "05b"], async () => {
      await begin(f);
      await progress(f.watcher, options);
      const before = await snapshot(f.owner);
      await nativeEvaluate(f.owner, () => {
        window.__e2e.savedTrack = window.__e2e.captures.find(
          (c) => c.track.readyState === "live",
        ).track;
        window.__e2e.savedPeer = window.__e2e.peers.find(
          (p) => p.connectionState === "connected",
        );
      });
      const interruptedSockets = await interrupt(f.owner, plane);
      // Wait for a replacement signaling socket before measuring resumed frames.
      await until(
        () => snapshot(f.owner),
        (s) =>
          s.sockets.filter((x) => x.plane === plane && x.ready === 1).length >
          0,
        "signaling-reconnect-deadline",
        20_000,
      );
      const after = await snapshot(f.owner);
      const identity = await nativeEvaluate(f.owner, () => ({
        trackLive: window.__e2e.savedTrack?.readyState === "live",
        trackStillSent: window.__e2e.peers.some(
          (p) =>
            p.connectionState !== "closed" &&
            p.getSenders().some((s) => s.track === window.__e2e.savedTrack),
        ),
        originalPeerConnected:
          window.__e2e.savedPeer?.connectionState === "connected",
      }));
      check(
        after.displayCalls === before.displayCalls &&
          identity.trackLive &&
          identity.trackStillSent,
        "recovery-recaptured-or-lost-track",
        { controlConfirmed: true, before, after, identity },
      );
      if (plane === "gateway")
        check(
          identity.originalPeerConnected &&
            after.peers.length === before.peers.length,
          "gateway-replaced-healthy-media",
          { controlConfirmed: true, before, after, identity },
        );
      const recovery = await progress(f.watcher, {
        ...options,
        budget: 20_000,
      });
      return { interruptedSockets, before, after, identity, recovery };
    });
  }
  await h.run("offline-8s-no-new-capture-gesture", ["02"], async () => {
    await begin(f);
    await progress(f.watcher, options);
    const before = await snapshot(f.owner);
    await f.owner.context.setOffline(true);
    try {
      await observe(8_000, () => snapshot(f.owner));
    } finally {
      await f.owner.context.setOffline(false);
    }
    const frames = await progress(f.watcher, { ...options, budget: 20_000 });
    const after = await snapshot(f.owner);
    check(
      after.displayCalls === before.displayCalls &&
        after.captures.filter((c) => c.state === "live").length === 1,
      "offline-capture-lost-or-reprompted",
      { before, after },
    );
    return {
      offlineMs: 8_000,
      before,
      after,
      frames,
      limitation:
        "browser offline can leave established UDP flowing; Media-WS fault is tested separately",
    };
  });
  await h.run(
    "watch-navigation-channel-binding-global-controls",
    ["08a"],
    async () => {
      await begin(f);
      await progress(f.watcher, options);
      // SPA navigation is crucial: page.goto would hide the bug by destroying the session.
      await f.watcher.page
        .getByRole("link", { name: "E2E Voice B", exact: true })
        .click();
      const foreign = await snapshot(f.watcher);
      check(
        !foreign.videos.some((v) => v.kind === "live" && v.width > 0),
        "old-channel-live-rendered-under-new-channel",
        foreign,
      );
      const stop = f.watcher.page.getByRole("button", {
        name: "Nicht mehr zuschauen",
        exact: true,
      });
      check((await stop.count()) > 0, "global-watch-stop-unavailable", {
        controlConfirmed: true,
        watcherAfterNavigation: foreign,
      });
      await stop.first().click();
      await f.owner.page
        .getByRole("link", { name: "E2E Voice B", exact: true })
        .click();
      for (const name of ["Mikrofon aus", "Live beenden", "Verlassen"])
        check(
          (await f.owner.page
            .getByRole("button", { name, exact: true })
            .count()) > 0,
          "global-voice-control-unavailable",
        );
      await click(f.owner, "Mikrofon aus");
      await click(f.owner, "Live beenden");
      await click(f.owner, "Verlassen");
      return {
        watcherAfterNavigation: foreign,
        ownerAfterLeave: await snapshot(f.owner),
      };
    },
  );
  await h.run(
    "live-claim-exclusive-and-loser-not-published",
    ["05b", "03b", "08b"],
    async () => {
      const { concurrentClaim } = await import("./media-extra.mjs");
      return concurrentClaim(h, f, { reset, options });
    },
  );
  await h.run(
    "20-live-start-stop-bounded-client-resources",
    ["01", "02", "05b"],
    async () => {
      await reset(f);
      await click(f.owner, "Beitreten");
      const cycles = [];
      for (let i = 0; i < 20; i++) {
        await click(f.owner, "Go Live");
        await click(f.watcher, "Zuschauen");
        const frames = await progress(f.watcher, {
          ...options,
          color: i % 2 ? [30, 220, 30] : [220, 30, 30],
        });
        await click(f.owner, "Live beenden");
        await until(
          () => snapshot(f.owner),
          (s) => s.captures.every((c) => c.state === "ended"),
          "stopped-capture-still-live",
        );
        const owner = await snapshot(f.owner),
          watcher = await snapshot(f.watcher);
        cycles.push({
          cycle: i + 1,
          owner,
          watcher,
          decodedFrames: decoded(frames.last),
        });
        // Explicit watcher stop is UI cleanup; the next start never reloads publisher.
        const stop = f.watcher.page.getByRole("button", {
          name: "Nicht mehr zuschauen",
          exact: true,
        });
        if (await stop.count()) await stop.first().click();
      }
      await click(f.owner, "Go Live");
      const late = await h.actor("CycleWatch");
      await f.join(late);
      await click(late, "Zuschauen");
      const last = await progress(late, options);
      check(
        activePeers(last.last)
          .flatMap((p) => p.inbound)
          .filter((r) => r.kind === "video").length === 1,
        "late-watch-received-stale-publications",
        last.last,
      );
      check(
        cycles.every(
          (c) =>
            activePeers(c.owner).length <= 1 &&
            activePeers(c.owner).every(
              (p) =>
                p.transceivers <= 4 &&
                p.senders <= 1 &&
                p.localSdpBytes <= 32_000,
            ),
        ),
        "publisher-resource-growth",
        { controlConfirmed: true, cycles, late: last },
      );
      check(
        cycles.every(
          (c) =>
            activePeers(c.watcher).length <= 1 &&
            activePeers(c.watcher).every(
              (p) => p.transceivers <= 4 && p.receivers <= 2,
            ),
        ),
        "watch-resource-growth",
        { cycles, late: last },
      );
      return {
        cycles,
        late: last,
        serverBounds: "BLOCKED: SFU task/publication counters not exposed",
      };
    },
  );
  await h.run(
    "cancelled-live-picker-keeps-voice-no-ghost",
    ["02", "05b"],
    async () => {
      await reset(f);
      await click(f.owner, "Beitreten");
      await nativeEvaluate(f.owner, () => {
        window.__e2e.cancelNextCapture = true;
      });
      await click(f.owner, "Go Live");
      const result = await observe(2_000, () => snapshot(f.owner));
      check(
        result.captures.every((c) => c.state !== "live") &&
          (await f.owner.page
            .getByRole("button", { name: "Go Live", exact: true })
            .count()) > 0,
        "cancelled-picker-ghost-live",
        result,
      );
      check(
        (await f.owner.page
          .getByRole("button", { name: "Verlassen", exact: true })
          .count()) > 0,
        "cancelled-picker-lost-voice",
      );
      return result;
    },
  );
  await h.run("camera-screen-live-source-identity", ["01", "08b"], async () => {
    await begin(f);
    await click(f.member, "Beitreten");
    await click(f.owner, "Kamera an");
    await click(f.owner, "Bildschirm teilen");
    await watchSource(f.member, "live");
    await watchSource(f.member, "screen");
    const live = await progress(f.member, options);
    const screen = await progress(f.member, {
      ...options,
      kind: "screen",
      color: [30, 220, 30],
    });
    const camera = await progress(f.member, {
      ...options,
      kind: "camera",
      color: [30, 60, 220],
    });
    return {
      live,
      screen,
      camera,
      limitation: "arrival reorder not forced; natural native negotiation only",
    };
  });
  await h.run("autoplay-rejection-visible-click-retry", ["08a"], async () => {
    await reset(f);
    await click(f.owner, "Beitreten");
    await click(f.owner, "Go Live");
    await nativeEvaluate(f.watcher, () => {
      window.__e2e.rejectPlayback = true;
    });
    await click(f.watcher, "Zuschauen");
    await until(
      () => snapshot(f.watcher),
      (s) => s.playRejected > 0,
      "blocked-play-fault-not-exercised",
    );
    const retry = f.watcher.page.getByRole("button", {
      name: /Wiedergabe starten|Ton starten|Abspielen|Wiedergabe wiederholen|Play/i,
    });
    await retry.first().waitFor({ state: "visible", timeout: 5_000 });
    check(
      (await retry.count()) > 0,
      "blocked-play-has-no-visible-retry",
      await snapshot(f.watcher),
    );
    await nativeEvaluate(f.watcher, armPlaybackRetry);
    await retry.first().click();
    return {
      fault:
        "synthetic NotAllowedError; normal browser policy tested separately",
      frames: await progress(f.watcher, options),
    };
  });
  const { mediaExtraScenarios } = await import("./media-extra.mjs");
  await h.run("active-call-responsive-controls", [], async () => {
    await reset(f);
    const page = f.owner.page;
    const originalViewport = page.viewportSize();
    const results = [];
    try {
      await page.setViewportSize({ width: 1487, height: 1058 });
      await click(f.owner, "Beitreten");
      await page.getByRole("region", { name: "Aktive Medien" }).waitFor();
      check(
        (await page
          .getByRole("button", { name: "Mikrofon aus", exact: true })
          .count()) === 1,
        "duplicate-in-room-call-controls",
      );
      await page.locator(`a[href="${f.textPath}"]`).first().click();
      await page.locator("textarea").waitFor();
      for (const viewport of [
        { width: 1487, height: 1058 },
        { width: 1366, height: 600 },
        { width: 390, height: 844 },
        { width: 780, height: 390 },
        { width: 844, height: 390 },
      ]) {
        await page.setViewportSize(viewport);
        const metrics = await until(
          () =>
            nativeEvaluate(f.owner, () => {
              const send = [...document.querySelectorAll("button")].find(
                (button) => button.textContent.trim() === "Senden",
              );
              const rect = send.getBoundingClientRect();
              const hit = document.elementFromPoint(
                rect.x + rect.width / 2,
                rect.y + rect.height / 2,
              );
              const dock = document.querySelector(".voice-session-dock");
              const dockBounds = dock.getBoundingClientRect();
              const callControlsUncovered = [
                ...dock.querySelectorAll("button"),
              ].every((button) => {
                const bounds = button.getBoundingClientRect();
                const target = document.elementFromPoint(
                  bounds.x + bounds.width / 2,
                  bounds.y + bounds.height / 2,
                );
                return target === button || button.contains(target);
              });
              return {
                sendUncovered: hit === send || send.contains(hit),
                callControlsUncovered,
                dockAtBottom:
                  Math.abs(dockBounds.bottom - window.innerHeight) <= 1,
                overflow: document.documentElement.scrollWidth > innerWidth,
                channelsHeight: document
                  .querySelector(".sidebar-list-scroll")
                  .getBoundingClientRect().height,
              };
            }),
          (state) =>
            state.sendUncovered &&
            state.callControlsUncovered &&
            state.dockAtBottom &&
            !state.overflow,
          "call-dock-covers-chat-controls",
        );
        if (viewport.width <= 800) {
          await page.getByRole("button", { name: "Navigation öffnen" }).click();
        } else {
          check(metrics.channelsHeight >= 156, "channel-navigation-collapsed");
        }
        // Scroll to the owner's controls, including on short landscape drawers.
        await page
          .getByRole("button", { name: "Kanal erstellen", exact: true })
          .click();
        await page
          .getByRole("dialog", { name: "Kanal erstellen", exact: true })
          .waitFor();
        await page.keyboard.press("Escape");
        if (viewport.width <= 800) {
          check(
            await page
              .getByRole("dialog", { name: "Navigation", exact: true })
              .isVisible(),
            "nested-escape-closed-navigation",
          );
          await page.keyboard.press("Escape");
        }
        results.push({ viewport, ...metrics });
      }
      return { viewports: results };
    } finally {
      if (originalViewport) await page.setViewportSize(originalViewport);
    }
  });
  await mediaExtraScenarios(h, f, { reset, begin, options });
  h.blocked(
    "audible-voice-mute-deafen-quality",
    "requires two devices and human listening; existing relay audio smoke retained",
  );
  h.blocked(
    "sfu-publication-task-bounds",
    "no read-only per-test-room SFU counters",
    ["01"],
  );
  const { leaseFaultScenarios } = await import("./media-lease-faults.mjs");
  await leaseFaultScenarios(h, options);
  h.blocked(
    "packet-loss-audible-quality",
    "selected-UDP kernel harness reviewed; full remote netem and audible quality remain open",
  );
}
