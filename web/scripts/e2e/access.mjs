/* global window, URL, fetch, setTimeout, clearTimeout */
import {
  nativeEvaluate,
  NativeInterfaceFailure,
  deadlineProbe,
} from "./native-evaluate.mjs";
import {
  api as harnessApi,
  check,
  click,
  navigate,
  observe,
  until,
} from "./harness.mjs";
import { progress } from "./media.mjs";
import { attemptAll } from "./teardown.mjs";
const api = (actor, path, method, body) =>
  harnessApi(actor, path, method, body, 10_000);
async function restoreOwnerAccount(actor, base) {
  const current = await api(actor, "/auth/session");
  if (current.body.user?.id === actor.id) return;
  if (current.body.user) await api(actor, "/auth/logout", "POST");
  await navigate(actor, "/login", base);
  await actor.page.getByLabel("E-Mail-Adresse").fill(actor.email);
  await actor.page.getByLabel("Passwort", { exact: true }).fill(actor.password);
  await click(actor, "Anmelden");
  await actor.page.waitForURL((u) => !u.pathname.includes("login"));
  check(
    (await api(actor, "/auth/session")).body.user?.id === actor.id,
    "fixture-account-restoration-failed",
  );
}

// A second native socket intentionally ignores UI auth cleanup. Store only
// counts/known protocol outcomes; private event bodies never leave the browser.
async function maliciousGateway(actor, serverId, channelId) {
  await nativeEvaluate(
    actor,
    async ({ serverId, channelId }) => {
      const state = window.__e2e;
      const url = new URL("/ws", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      const ws = new state.NativeSocket(url);
      const probe = (state.rawGateway = { ws, events: 0, subscribed: false });
      ws.addEventListener("message", (event) => {
        const frame = JSON.parse(event.data);
        if (frame.op === "h") ws.send('{"op":"h"}');
        if (frame.op === "ok" && frame.c === channelId) probe.subscribed = true;
        if (frame.op === "e" && frame.c === channelId) probe.events++;
      });
      await new Promise((resolve, reject) => {
        ws.addEventListener("open", resolve, { once: true });
        ws.addEventListener("error", reject, { once: true });
      });
      ws.send(JSON.stringify({ op: "s", s: serverId, c: channelId }));
    },
    { serverId, channelId },
  );
  await until(
    () =>
      nativeEvaluate(actor, () => ({
        subscribed: window.__e2e.rawGateway.subscribed,
      })),
    (s) => s.subscribed,
    "malicious-gateway-not-subscribed",
  );
}
async function rawSample(actor) {
  return nativeEvaluate(actor, () => ({
    state: window.__e2e.rawGateway.ws.readyState,
    events: window.__e2e.rawGateway.events,
  }));
}
async function heldTicket(actor, channelId) {
  // The ticket stays in browser memory, never in report or command output.
  const result = await nativeEvaluate(
    actor,
    async (channelId) => {
      const session = await fetch("/api/auth/session").then((r) => r.json());
      const r = await fetch(`/api/channels/${channelId}/media-ticket`, {
        method: "POST",
        headers: { "X-CSRF-Token": session.csrf_token },
      });
      if (r.status === 200) window.__e2e.heldTicket = await r.json();
      return { status: r.status };
    },
    channelId,
  );
  check(result.status === 200, "pre-revocation-ticket-not-issued", result);
}
async function useHeldTicket(actor) {
  await nativeEvaluate(actor, async () => {
    const state = window.__e2e,
      ticket = state.heldTicket;
    const url = new URL(ticket.media_path, window.location.href);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const ws = new state.NativeSocket(url);
    const probe = (state.rawTicket = { ws, accepted: false, denied: false });
    ws.addEventListener("message", (event) => {
      const f = JSON.parse(event.data);
      if (f.op === "ok") probe.accepted = true;
      if (f.op === "err")
        probe.denied = ["unauthorized", "gone", "forbidden"].includes(f.e);
    });
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    ws.send(JSON.stringify({ op: "j", tk: ticket.ticket }));
  });
  const outcome = await until(
    () =>
      nativeEvaluate(actor, () => ({
        accepted: window.__e2e.rawTicket.accepted,
        denied: window.__e2e.rawTicket.denied,
        state: window.__e2e.rawTicket.ws.readyState,
      })),
    (s) => s.accepted || s.denied || s.state === 3,
    "held-ticket-no-result",
    5_000,
  );
  check(
    !outcome.accepted && (outcome.denied || outcome.state === 3),
    "revoked-unconsumed-ticket-accepted",
    outcome,
  );
  return outcome;
}
export async function holdActiveMedia(actor) {
  await nativeEvaluate(actor, () => {
    const state = window.__e2e;
    state.heldFrameCounterKeys = null;
    state.heldPeers = state.peers.filter(
      (p) => p.connectionState === "connected",
    );
    state.heldSockets = state.sockets.filter(
      (s) => s.plane === "media" && s.ws.readyState === 1,
    );
    state.restoreClose = [];
    // Ignore client-side teardown to test the SFU boundary independently.
    for (const pc of state.heldPeers) {
      state.restoreClose.push(() =>
        window.RTCPeerConnection.prototype.close.call(pc),
      );
      pc.close = () => {};
    }
    for (const { ws } of state.heldSockets) {
      state.restoreClose.push(() =>
        state.NativeSocket.prototype.close.call(ws),
      );
      ws.close = () => {};
    }
  });
  const baseline = await heldMedia(actor);
  check(
    baseline.frames > 0,
    "fixture-held-positive-decoded-counter-missing",
    baseline,
  );
  return baseline;
}
export async function heldMedia(actor) {
  const measurement = await nativeEvaluate(
    actor,
    async function sample({ deadlineEpochMs }) {
      const state = window.__e2e;
      let frames = 0,
        videoRtpEntries = 0;
      const availableKeys = [];
      for (const [peerIndex, pc] of state.heldPeers.entries()) {
        window.__e2e.samplePhase = "native-getStats";
        let timer;
        let stats;
        try {
          const remaining = deadlineEpochMs - Date.now();
          if (remaining <= 0) throw new Error("E2E_NATIVE_STATS_DEADLINE");
          stats = await Promise.race([
            pc.getStats(),
            new Promise((_, reject) => {
              timer = setTimeout(
                () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
                Math.max(
                  0,
                  remaining - Math.min(100, Math.max(1, remaining / 10)),
                ),
              );
            }),
          ]);
        } finally {
          clearTimeout(timer);
        }
        window.__e2e.samplePhase = "native-stats-resolved";
        for (const [key, r] of stats.entries())
          if (r.type === "inbound-rtp" && (r.kind ?? r.mediaType) === "video") {
            videoRtpEntries++;
            if (Number.isInteger(r.framesDecoded) && r.framesDecoded >= 0) {
              availableKeys.push(`${peerIndex}:${key}`);
              frames += r.framesDecoded;
            }
          }
      }
      // Counter IDs stay in browser memory. A disappearing/replaced report cannot
      // reduce the observed sum and then masquerade as stagnant decoded media.
      const complete =
        videoRtpEntries > 0 && availableKeys.length === videoRtpEntries;
      if (complete && !state.heldFrameCounterKeys)
        state.heldFrameCounterKeys = availableKeys;
      const expectedKeys = state.heldFrameCounterKeys ?? [];
      const frameCountersAvailable =
        complete &&
        expectedKeys.length === availableKeys.length &&
        expectedKeys.every((key) => availableKeys.includes(key));
      return {
        frames: frameCountersAvailable ? frames : null,
        frameCountersAvailable,
        videoRtpEntries,
        availableFrameCounters: availableKeys.length,
        expectedFrameCounters: expectedKeys.length,
        openSockets: window.__e2e.heldSockets.filter(
          (s) => s.ws.readyState === 1,
        ).length,
      };
    },
  );
  check(
    measurement.frameCountersAvailable,
    "fixture-held-video-counters-unavailable",
    measurement,
  );
  return measurement;
}
export async function releaseHeld(actor) {
  if (actor.nativeEvaluationUnusable || actor.page.isClosed?.()) return;
  await nativeEvaluate(actor, () => {
    const state = window.__e2e;
    for (const restore of state.restoreClose ?? []) restore();
    state.rawGateway?.ws.close();
    state.rawTicket?.ws.close();
  }).catch((error) => {
    if (error instanceof NativeInterfaceFailure) throw error;
    if (!actor.page.isClosed?.()) throw error;
  });
}
export async function closeAccessActors(
  actors,
  restoreActors = [],
  budgetMs = 5_000,
) {
  const failures = await attemptAll([
    ...restoreActors.map((actor, i) => [
      `held-restore-${i}`,
      () => releaseHeld(actor),
    ]),
    ...actors.map((actor, i) => [
      `context-close-${i}`,
      () => deadlineProbe(() => actor.context.close(), Date.now() + budgetMs),
    ]),
  ]);
  if (failures.length)
    throw new NativeInterfaceFailure({
      stage: "access-restore-or-context-close",
      failedSteps: failures,
      nativeDataAvailable: false,
    });
}
async function closeIndependentSession(page, context) {
  const failedSteps = await attemptAll([
    ...(page
      ? [
          [
            "independent-session-logout",
            async () => {
              const logout = await api({ page }, "/auth/logout", "POST");
              check(
                logout.status === 200,
                "independent-session-cleanup-logout-failed",
                { status: logout.status },
              );
            },
          ],
        ]
      : []),
    ["independent-context-close", () => closeAccessActors([{ context }])],
  ]);
  if (failedSteps.length)
    throw new NativeInterfaceFailure({
      stage: "independent-session-owned-cleanup",
      failedSteps,
      nativeDataAvailable: false,
    });
}
export async function accessScenarios(h, f) {
  h.setIsolation(() => restoreOwnerAccount(f.owner, f.base));
  const chat = f.textPath.split("/").at(-1),
    voice = f.voicePath.split("/").at(-1);
  for (const mode of ["leave", "kick", "ban", "logout"]) {
    await h.run(
      `${mode}-revokes-existing-gateway-active-sfu-and-held-ticket`,
      ["03a", "03b"],
      async () => {
        const victim = await h.actor(`RevokedWatch${mode}`);
        try {
          await f.join(victim);
          await navigate(f.owner, f.voicePath, f.base);
          await click(f.owner, "Beitreten");
          await click(f.owner, "Go Live");
          await click(victim, "Zuschauen");
          await progress(victim, {
            relay: h.relay,
            budget: h.relay ? 10_000 : 5_000,
          });
          await maliciousGateway(victim, f.serverId, chat);
          const gatewayControlBefore = await rawSample(victim);
          const controlMessage = await api(
            f.owner,
            `/channels/${chat}/messages`,
            "POST",
            { content: "E2E pre-revocation control event", attachment_ids: [] },
          );
          check(
            [200, 201].includes(controlMessage.status),
            "control-message-create-failed",
            { status: controlMessage.status },
          );
          const gatewayControl = await until(
            () => rawSample(victim),
            (s) => s.events > gatewayControlBefore.events,
            "pre-revocation-gateway-control-failed",
          );
          await heldTicket(victim, voice);
          await holdActiveMedia(victim);
          const baseline = await heldMedia(victim);
          let revoked;
          if (mode === "leave")
            revoked = await api(victim, `/servers/${f.serverId}/leave`, "POST");
          else if (mode === "logout")
            revoked = await api(victim, "/auth/logout", "POST");
          else
            revoked = await api(
              f.owner,
              `/servers/${f.serverId}/${mode}`,
              "POST",
              { user_id: victim.id },
            );
          check(
            revoked.status === 204 || revoked.status === 200,
            "revocation-request-failed",
            { status: revoked.status },
          );
          // Wait a finite revocation budget; then sample a second interval for continued video.
          const settled = await observe(3_000, () => heldMedia(victim));
          const after = await observe(1_000, () => heldMedia(victim));
          const beforeGateway = await rawSample(victim);
          const message = await api(
            f.owner,
            `/channels/${chat}/messages`,
            "POST",
            {
              content: "E2E post-revocation synthetic event",
              attachment_ids: [],
            },
          );
          check(
            message.status === 201 || message.status === 200,
            "revocation-event-fixture-failed",
            { status: message.status },
          );
          const gateway = await observe(1_000, () => rawSample(victim));
          const rest = await api(victim, `/channels/${chat}/messages`);
          const freshTicket = await api(
            victim,
            `/channels/${voice}/media-ticket`,
            "POST",
          );
          // Evaluate every boundary, even when one is red; do not short circuit away the ticket attack.
          let held;
          try {
            held = await useHeldTicket(victim);
          } catch (error) {
            if (error instanceof NativeInterfaceFailure) throw error;
            held = {
              accepted: error.metrics?.accepted ?? null,
              denied: error.metrics?.denied ?? false,
              failed: true,
            };
          }
          const metrics = {
            controlConfirmed:
              after.frames > settled.frames ||
              gateway.events > beforeGateway.events ||
              held.accepted === true ||
              rest.status === 200 ||
              freshTicket.status === 200,
            gatewayControl,
            revocationStatus: revoked.status,
            baseline,
            settled,
            after,
            beforeGateway,
            gateway,
            restStatus: rest.status,
            freshTicketStatus: freshTicket.status,
            heldTicket: held,
            clientTeardownSuppressed: true,
          };
          check(
            after.frames === settled.frames &&
              after.openSockets === 0 &&
              gateway.events === beforeGateway.events &&
              [401, 403, 404].includes(rest.status) &&
              [401, 403, 404].includes(freshTicket.status) &&
              !held.failed,
            "revoked-client-retained-access",
            metrics,
          );
          return metrics;
        } finally {
          await closeAccessActors([victim], [victim]);
        }
      },
    );
  }
  await h.run(
    "logout-other-independent-session-survives",
    ["03a"],
    async () => {
      const context = await deadlineProbe(
        () => h.browser.newContext(),
        Date.now() + 12_000,
      );
      let page;
      try {
        page = await deadlineProbe(() => context.newPage(), Date.now() + 5_000);
        await page.goto(`${f.base}/login`);
        await page.getByLabel("E-Mail-Adresse").fill(f.owner.email);
        await page
          .getByLabel("Passwort", { exact: true })
          .fill(f.owner.password);
        await page
          .getByRole("button", { name: "Anmelden", exact: true })
          .click();
        await page.waitForURL((u) => !u.pathname.includes("login"));
        const before = await api({ page }, "/auth/session");
        check(
          before.status === 200 && before.body?.user?.id === f.owner.id,
          "fixture-independent-session-not-authenticated",
          { status: before.status },
        );
        const ownerLogout = await api(f.owner, "/auth/logout", "POST");
        check(ownerLogout.status === 200, "owner-logout-request-failed", {
          status: ownerLogout.status,
        });
        const ownerSession = await api(f.owner, "/auth/session");
        check(
          ownerSession.status === 200 && ownerSession.body?.user === null,
          "owner-session-not-anonymous-after-logout",
          { status: ownerSession.status },
        );
        const after = await api({ page }, "/auth/session");
        check(
          after.status === 200 && after.body?.user?.id === f.owner.id,
          "logout-killed-other-session",
          { status: after.status },
        );
        return {
          independentSessionRetained: true,
          ownerLogoutStatus: ownerLogout.status,
          ownerSessionAnonymousStatus: ownerSession.status,
          independentSessionStatus: after.status,
        };
      } finally {
        await closeIndependentSession(page, context);
      }
    },
  );
  await h.run(
    "channel-server-delete-active-sockets",
    ["03a", "03b"],
    async () => {
      const controls = [];
      for (const scope of ["channel", "server"]) {
        // A separate owned fixture prevents server deletion contaminating later cases.
        const owned = await h.fixture();
        const victim = owned.watcher;
        const voiceId = owned.voicePath.split("/").at(-1);
        const chatId = owned.textPath.split("/").at(-1);
        try {
          await click(owned.owner, "Beitreten");
          await click(owned.owner, "Go Live");
          await click(victim, "Zuschauen");
          await progress(victim, {
            relay: h.relay,
            budget: h.relay ? 10_000 : 5_000,
          });
          await maliciousGateway(victim, owned.serverId, chatId);
          if (scope === "server") {
            const before = await rawSample(victim);
            const seed = await api(
              owned.owner,
              `/channels/${chatId}/messages`,
              "POST",
              { content: "E2E delete-server native control" },
            );
            check(seed.status === 201, "fixture-delete-server-control-failed");
            await until(
              () => rawSample(victim),
              (s) => s.events > before.events,
              "fixture-delete-server-event-not-observed",
            );
          }
          await heldTicket(victim, voiceId);
          await holdActiveMedia(victim);
          const removed =
            scope === "server"
              ? await api(owned.owner, `/servers/${owned.serverId}`, "DELETE")
              : await api(owned.owner, `/channels/${voiceId}`, "DELETE");
          check(removed.status === 204, "fixture-owned-room-delete-failed", {
            status: removed.status,
          });
          if (scope === "server") h.markServerDeleted(owned.serverId);
          const settled = await observe(3_000, () => heldMedia(victim));
          const after = await observe(1_000, () => heldMedia(victim));
          const fresh = await api(
            victim,
            `/channels/${voiceId}/media-ticket`,
            "POST",
          );
          const held = await useHeldTicket(victim);
          check(
            after.frames === settled.frames &&
              after.openSockets === 0 &&
              [403, 404].includes(fresh.status) &&
              !held.accepted,
            "deleted-room-retained-active-media-or-ticket-access",
            {
              controlConfirmed:
                after.frames > settled.frames ||
                fresh.status === 200 ||
                held.accepted,
              scope,
              settled,
              after,
              freshTicketStatus: fresh.status,
              heldTicket: held,
            },
          );
          if (scope === "server") {
            const rest = await api(victim, `/channels/${chatId}/messages`);
            check(
              [403, 404].includes(rest.status),
              "deleted-server-retained-chat-access",
              { controlConfirmed: true, status: rest.status },
            );
          } else {
            // Deleting one voice channel must preserve the remaining text channel.
            check(
              (await api(victim, `/channels/${chatId}/messages`)).status ===
                200,
              "channel-delete-revoked-unrelated-text-channel",
            );
          }
          controls.push({
            scope,
            deleteStatus: removed.status,
            activeDecodedControl: true,
            clientTeardownSuppressed: true,
            framesStopped: true,
            mediaSocketClosed: true,
            heldTicketRejected: true,
            freshTicketStatus: fresh.status,
          });
        } finally {
          await closeAccessActors(
            [owned.owner, owned.member, victim],
            [victim],
          );
        }
      }
      return { controls };
    },
  );
  h.setIsolation(null);
}
