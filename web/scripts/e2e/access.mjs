/* global window, URL, fetch */
import { api, check, click, navigate, observe, until } from "./harness.mjs";
import { progress } from "./media.mjs";

// A second native socket intentionally ignores UI auth cleanup. Store only
// counts/known protocol outcomes; private event bodies never leave the browser.
async function maliciousGateway(actor, serverId, channelId) {
  await actor.page.evaluate(
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
      actor.page.evaluate(() => ({
        subscribed: window.__e2e.rawGateway.subscribed,
      })),
    (s) => s.subscribed,
    "malicious-gateway-not-subscribed",
  );
}
async function rawSample(actor) {
  return actor.page.evaluate(() => ({
    state: window.__e2e.rawGateway.ws.readyState,
    events: window.__e2e.rawGateway.events,
  }));
}
async function heldTicket(actor, channelId) {
  // The ticket stays in browser memory, never in report or command output.
  const result = await actor.page.evaluate(async (channelId) => {
    const session = await fetch("/api/auth/session").then((r) => r.json());
    const r = await fetch(`/api/channels/${channelId}/media-ticket`, {
      method: "POST",
      headers: { "X-CSRF-Token": session.csrf_token },
    });
    if (r.status === 200) window.__e2e.heldTicket = await r.json();
    return { status: r.status };
  }, channelId);
  check(result.status === 200, "pre-revocation-ticket-not-issued", result);
}
async function useHeldTicket(actor) {
  await actor.page.evaluate(async () => {
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
      actor.page.evaluate(() => ({
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
async function holdActiveMedia(actor) {
  await actor.page.evaluate(() => {
    const state = window.__e2e;
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
}
async function heldMedia(actor) {
  return actor.page.evaluate(async () => {
    let frames = 0;
    for (const pc of window.__e2e.heldPeers) {
      const stats = await pc.getStats();
      for (const r of stats.values())
        if (r.type === "inbound-rtp" && (r.kind ?? r.mediaType) === "video")
          frames += r.framesDecoded ?? 0;
    }
    return {
      frames,
      openSockets: window.__e2e.heldSockets.filter((s) => s.ws.readyState === 1)
        .length,
    };
  });
}
async function releaseHeld(actor) {
  await actor.page
    .evaluate(() => {
      const state = window.__e2e;
      for (const restore of state.restoreClose ?? []) restore();
      state.rawGateway?.ws.close();
      state.rawTicket?.ws.close();
    })
    .catch(() => {});
}
export async function accessScenarios(h, f) {
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
          await releaseHeld(victim);
          await victim.context.close();
        }
      },
    );
  }
  await h.run(
    "logout-other-independent-session-survives",
    ["03a"],
    async () => {
      const context = await h.browser.newContext(),
        page = await context.newPage();
      try {
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
        await api(f.owner, "/auth/logout", "POST");
        const after = await api({ page }, "/auth/session");
        check(
          before.body.user?.id === f.owner.id &&
            after.body.user?.id === f.owner.id,
          "logout-killed-other-session",
        );
        await api({ page }, "/auth/logout", "POST");
        return { independentSessionRetained: true };
      } finally {
        await context.close();
      }
    },
  );
  h.blocked(
    "ban-invite-join-race",
    "03a lock race deterministically covered by backend; browser concurrency alone cannot force transaction overlap",
    ["03a"],
  );
  h.blocked(
    "channel-server-delete-active-sockets",
    "negative deleted-room scenario remains for integrated 03b",
    ["03b"],
  );
  h.blocked(
    "slow-reader-bounded-memory",
    "requires isolated server metrics and controllable raw TCP backpressure",
    ["05a"],
  );
}
