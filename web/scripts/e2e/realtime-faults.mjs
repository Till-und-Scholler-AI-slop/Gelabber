/* global window, URL */
import {
  api,
  check,
  navigate,
  observe,
  snapshot,
  until,
  CheckFailure,
} from "./harness.mjs";
import { nativeEvaluate } from "./native-evaluate.mjs";
const log = (a) => a.page.getByRole("log", { name: "Nachrichten" });
const counter = (s, key) =>
  s.sockets
    .filter((x) => x.plane === "gateway")
    .reduce((n, x) => n + (x[key] ?? 0), 0);
async function seed(a, id, content) {
  const r = await api(a, `/channels/${id}/messages`, "POST", { content });
  check(r.status === 201, "fixture-outbox-message-commit-failed", {
    status: r.status,
  });
  return r.body;
}
async function fresh(f, name) {
  await navigate(f.owner, f.textPath, f.base);
  const path = await f.channel(name, "Text");
  for (const a of [f.owner, f.member]) await navigate(a, path, f.base);
  return { path, id: path.split("/").at(-1) };
}
async function visible(a, content) {
  const escaped = content.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  await log(a)
    .locator("p")
    .filter({ hasText: new RegExp(`^${escaped}(?:\\s*\\(bearbeitet\\))?$`) })
    .waitFor();
}
export async function realtimeFaultScenarios(h, f, runtime) {
  if (!runtime) {
    for (const id of [
      "redis-epoch-replay-window-reset",
      "redis-pubsub-outage-durable-outbox-convergence",
    ])
      h.blocked(
        id,
        "requires owned API/DB and connection-scoped Redis proxy; shared Redis cannot be flushed",
        ["05a", "06b"],
      );
  } else {
    await h.run("redis-epoch-replay-window-reset", ["05a", "06b"], async () => {
      let stage = "fresh-channel";
      let outage;
      try {
        const c = await fresh(f, "E2E replay and epoch");
        runtime.allowTopic(c.id);
        const control = await seed(f.owner, c.id, "E2E before replay control");
        await visible(f.member, control.content);
        const before = await snapshot(f.member);
        stage = "disconnect-and-overflow";
        check(
          before.sockets.some((s) => s.plane === "gateway" && s.ready === 1),
          "replay-control-live-gateway-missing",
        );
        try {
          // Reconnect starts on the next turn. Block new network connections
          // before closing the old one, including browsers whose offline mode
          // leaves existing WebSockets alive. Offline may itself close it.
          await f.member.context.setOffline(true);
          await nativeEvaluate(f.member, () => {
            for (const s of window.__e2e.sockets)
              if (s.plane === "gateway" && s.ws.readyState === 1)
                s.ws.close(4000, "e2e-replay-offline");
          });
          const disconnected = await until(
            () => snapshot(f.member),
            (s) =>
              !s.sockets.some((x) => x.plane === "gateway" && x.ready === 1),
            "replay-control-gateway-still-open",
          );
          for (let i = 0; i < 12; i++)
            await seed(f.owner, c.id, `E2E overflow ${i}`);
          await until(
            async () =>
              Number(
                await runtime.sql(
                  `SELECT count(*) FROM gateway_outbox WHERE channel_id='${c.id}'::uuid`,
                ),
              ),
            (n) => n === 0,
            "replay-control-outbox-not-delivered",
          );
          const offline = await snapshot(f.member);
          check(
            !offline.sockets.some(
              (s) => s.plane === "gateway" && s.ready === 1,
            ) &&
              counter(offline, "receivedEvents") ===
                counter(disconnected, "receivedEvents"),
            "replay-control-reconnected-before-overflow",
          );
          outage = {
            openBefore: before.sockets.filter(
              (s) => s.plane === "gateway" && s.ready === 1,
            ).length,
            openAfterWrites: offline.sockets.filter(
              (s) => s.plane === "gateway" && s.ready === 1,
            ).length,
            eventsAfterDisconnect: counter(disconnected, "receivedEvents"),
            eventsAfterWrites: counter(offline, "receivedEvents"),
            pendingOutboxBeforeReconnect: 0,
          };
        } finally {
          await f.member.context.setOffline(false);
        }
        stage = "replay-gap";
        await visible(f.member, "E2E overflow 11");
        const afterReplay = await snapshot(f.member);
        check(
          counter(afterReplay, "gaps") > counter(before, "gaps"),
          "replay-overflow-not-exercised",
          {
            gapsBefore: counter(before, "gaps"),
            gaps: counter(afterReplay, "gaps"),
            outage,
          },
        );
        // Only the topic created by this scenario. No FLUSHALL/FLUSHDB/global keys.
        stage = "own-topic-reset";
        await runtime.resetTopic(c.id);
        const edited = await api(f.owner, `/messages/${control.id}`, "PATCH", {
          content: "E2E after epoch edit",
        });
        check(
          edited.status === 200 && edited.body.revision > control.revision,
          "fixture-epoch-edit-contract-failed",
          { status: edited.status },
        );
        await visible(f.member, "E2E after epoch edit");
        await seed(f.owner, c.id, "E2E after epoch create");
        await visible(f.member, "E2E after epoch create");
        const after = await snapshot(f.member);
        check(
          counter(after, "epochChanges") >
            counter(afterReplay, "epochChanges") &&
            new URL(f.member.page.url()).pathname === c.path,
          "epoch-reset-control-or-convergence-missing",
          {
            controlConfirmed: true,
            epochChanges: counter(after, "epochChanges"),
          },
        );
        return {
          replayWindow: 8,
          missedEvents: 12,
          outage,
          gapsBefore: counter(before, "gaps"),
          gapsAfter: counter(afterReplay, "gaps"),
          gapObserved: true,
          ownTopicOnlyReset: true,
          disconnectedThroughoutOverflow: true,
          outboxDrainedBeforeReconnect: true,
          newEpochObserved: true,
          newerDatabaseRevision: true,
          editedAndCreatedWithoutReload: true,
        };
      } catch (error) {
        if (error instanceof CheckFailure) {
          error.metrics = { ...error.metrics, stage };
          throw error;
        }
        throw new CheckFailure("fixture-epoch-browser-interface", {
          stage,
          errorType: error?.name ?? "Error",
        });
      }
    });
    await h.run(
      "redis-pubsub-outage-durable-outbox-convergence",
      ["05a", "06b"],
      async () => {
        const c = await fresh(f, "E2E outbox outage");
        await seed(f.owner, c.id, "E2E before Redis outage");
        await visible(f.member, "E2E before Redis outage");
        await until(
          async () =>
            Number(
              await runtime.sql(
                `SELECT count(*) FROM gateway_outbox WHERE channel_id='${c.id}'::uuid`,
              ),
            ),
          (n) => n === 0,
          "fixture-positive-outbox-not-drained",
        );
        const before = await snapshot(f.member);
        let visibleDuringOutage;
        runtime.redis.block();
        try {
          const saved = await seed(
            f.owner,
            c.id,
            "E2E durable Redis outage commit",
          );
          check(
            (await runtime.sql(
              `SELECT count(*) FROM messages WHERE id='${saved.id}'::uuid`,
            )) === "1" &&
              Number(
                await runtime.sql(
                  `SELECT count(*) FROM gateway_outbox WHERE channel_id='${c.id}'::uuid`,
                ),
              ) > 0,
            "outage-http-success-without-durable-outbox",
            { controlConfirmed: true },
          );
          await observe(1_000, async () => ({
            interrupted: runtime.redis.counters.interrupted,
          }));
          visibleDuringOutage =
            (await log(f.member)
              .getByText(saved.content, { exact: true })
              .count()) > 0;
          const during = await snapshot(f.member);
          check(
            runtime.redis.counters.interrupted > 0 &&
              counter(during, "receivedEvents") ===
                counter(before, "receivedEvents"),
            "fixture-redis-fault-not-isolated-or-effective",
            {
              interrupted: runtime.redis.counters.interrupted,
              eventsBefore: counter(before, "receivedEvents"),
              eventsDuring: counter(during, "receivedEvents"),
              visibleViaRestDuringOutage: visibleDuringOutage,
            },
          );
        } finally {
          runtime.redis.restore();
        }
        await visible(f.member, "E2E durable Redis outage commit");
        const after = await until(
          () => snapshot(f.member),
          (s) => counter(s, "resyncs") > counter(before, "resyncs"),
          "pubsub-reconnect-resync-deadline",
          15_000,
        );
        check(
          counter(after, "resyncs") > counter(before, "resyncs"),
          "pubsub-reconnect-missed-resync",
          { controlConfirmed: true, resyncs: counter(after, "resyncs") },
        );
        await seed(f.owner, c.id, "E2E after outbox recovery");
        await visible(f.member, "E2E after outbox recovery");
        await until(
          async () =>
            Number(
              await runtime.sql(
                `SELECT count(*) FROM gateway_outbox WHERE channel_id='${c.id}'::uuid`,
              ),
            ),
          (n) => n === 0,
          "durable-outbox-not-drained-after-restore",
        );
        return {
          isolatedApiConnections: true,
          outageCommitStatus: 201,
          durableMessageAndOutboxObserved: true,
          resyncObserved: true,
          restoredWithoutNavigation: true,
          pendingOutboxAfterRestore: 0,
          visibleViaRestDuringOutage: visibleDuringOutage,
        };
      },
    );
  }
  await h.run(
    "private-dm-foreign-discovery-subscription-denied",
    ["05a", "06b"],
    async () => {
      await navigate(f.watcher, f.textPath, f.base);
      await f.watcher.page.evaluate(
        async ({ server, channel }) => {
          const url = new URL("/ws", window.location.href);
          url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
          const ws = new window.__e2e.NativeSocket(url);
          const probe = (window.__e2e.dmPrivacy = {
            ws,
            controlSubscribed: false,
            dmDiscoveries: 0,
            foreignSubscribed: false,
            denied: false,
          });
          ws.addEventListener("message", (e) => {
            const frame = JSON.parse(e.data);
            if (frame.op === "h") ws.send('{"op":"h"}');
            if (frame.op === "dm") probe.dmDiscoveries++;
            if (frame.op === "ok") {
              if (frame.c === channel) probe.controlSubscribed = true;
              else probe.foreignSubscribed = true;
            }
            if (
              frame.op === "err" &&
              ["not_found", "forbidden"].includes(frame.e)
            )
              probe.denied = true;
          });
          await new Promise((resolve, reject) => {
            ws.addEventListener("open", resolve, { once: true });
            ws.addEventListener("error", reject, { once: true });
          });
          ws.send(JSON.stringify({ op: "s", s: server, c: channel }));
        },
        { server: f.serverId, channel: f.textPath.split("/").at(-1) },
      );
      try {
        await until(
          () =>
            f.watcher.page.evaluate(
              () => window.__e2e.dmPrivacy.controlSubscribed,
            ),
          Boolean,
          "fixture-private-dm-control-subscription-missing",
        );
        const recipient = await h.actor("PrivateDMRecipient");
        const dm = await api(f.owner, "/dms", "POST", {
          user_id: recipient.id,
        });
        check(
          [200, 201].includes(dm.status),
          "fixture-private-dm-create-failed",
          { status: dm.status },
        );
        await seed(f.owner, dm.body.id, "E2E private DM privacy control");
        const forbiddenRest = await api(
          f.watcher,
          `/channels/${dm.body.id}/messages`,
        );
        check(
          [403, 404].includes(forbiddenRest.status),
          "foreign-dm-rest-access-allowed",
          { controlConfirmed: true, status: forbiddenRest.status },
        );
        await f.watcher.page.evaluate(
          (id) =>
            window.__e2e.dmPrivacy.ws.send(
              JSON.stringify({ op: "s", s: id, c: id }),
            ),
          dm.body.id,
        );
        const read = () =>
          f.watcher.page.evaluate(() => ({
            denied: window.__e2e.dmPrivacy.denied,
            foreignSubscribed: window.__e2e.dmPrivacy.foreignSubscribed,
            dmDiscoveries: window.__e2e.dmPrivacy.dmDiscoveries,
          }));
        const result = await until(
          read,
          (s) => s.denied || s.foreignSubscribed,
          "foreign-dm-subscription-no-result",
        );
        await observe(1_000, read);
        check(
          result.denied &&
            !result.foreignSubscribed &&
            (await read()).dmDiscoveries === 0,
          "foreign-dm-discovered-or-subscribed",
          { controlConfirmed: true, ...result },
        );
        return {
          authenticatedNormalTopicControl: true,
          foreignRestStatus: forbiddenRest.status,
          foreignSubscriptionDenied: true,
          foreignDiscoveryEvents: 0,
        };
      } finally {
        await f.watcher.page.evaluate(() => window.__e2e.dmPrivacy.ws.close());
      }
    },
  );
}
