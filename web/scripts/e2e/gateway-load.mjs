/* global URL */
import { spawn } from "node:child_process";
import { Buffer } from "node:buffer";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { setTimeout as pause } from "node:timers/promises";
import { api, check, navigate, until } from "./harness.mjs";

async function rawReader(f, channel, runtime) {
  const cookies = await f.watcher.context.cookies(f.base);
  const child = spawn(
    "python3",
    [fileURLToPath(new URL("./raw-reader.py", import.meta.url))],
    { stdio: ["pipe", "pipe", "ignore"] },
  );
  const lines = createInterface({ input: child.stdout })[
    Symbol.asyncIterator
  ]();
  async function reply() {
    const line = await Promise.race([
      lines.next(),
      pause(6_000, { done: true }, { ref: false }),
    ]);
    check(!line.done, "fixture-raw-reader-interface-deadline");
    const value = JSON.parse(line.value);
    check(!value.error, "fixture-raw-reader-interface-failed");
    return value;
  }
  async function close() {
    child.stdin.end('{"op":"close"}\n');
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      await Promise.race([exited, pause(2_000, undefined, { ref: false })]);
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await exited;
      }
    }
  }
  child.on("error", () => {});
  child.stdin.on("error", () => {});
  try {
    child.stdin.write(
      JSON.stringify({
        host: "127.0.0.1",
        port: Number(new URL(runtime.origin).port),
        server: f.serverId,
        channel,
        cookie: cookies.map((c) => `${c.name}=${c.value}`).join("; "),
      }) + "\n",
    );
    const ready = await reply();
    check(ready.subscribed, "fixture-raw-reader-subscription-missing");
    return {
      ready,
      reply,
      close,
      command: (op) => child.stdin.write(JSON.stringify({ op }) + "\n"),
    };
  } catch (error) {
    await close();
    throw error;
  }
}

export async function gatewayLoadScenarios(h, f, runtime) {
  const id = "slow-reader-bounded-memory";
  if (!runtime) {
    h.blocked(
      id,
      "requires this runner's own API process for raw TCP ownership and RSS sampling",
      ["05a"],
    );
    return;
  }
  await h.run(id, ["05a"], async () => {
    await navigate(f.owner, f.textPath, f.base);
    const path = await f.channel("E2E raw TCP backpressure", "Text");
    const channel = path.split("/").at(-1);
    await navigate(f.member, path, f.base);
    const raw = await rawReader(f, channel, runtime);
    try {
      raw.command("control");
      const control = await api(
        f.owner,
        `/channels/${channel}/messages`,
        "POST",
        { content: "E2E raw reader positive control" },
      );
      check(
        control.status === 201 && (await raw.reply()).positiveEvent,
        "fixture-raw-reader-event-control-failed",
      );
      await f.member.page
        .getByRole("log", { name: "Nachrichten" })
        .getByText("E2E raw reader positive control", { exact: true })
        .waitFor();
      raw.command("pause");
      check((await raw.reply()).paused, "fixture-raw-reader-not-paused");
      const pauseObservedAt = new Date().toISOString();
      const samples = [await runtime.processSample(raw.ready.port)];
      check(
        samples[0].rawSocketOpen,
        "fixture-raw-reader-not-owned-api-socket",
      );
      let sent = 0,
        bytes = 0,
        lastCommit = Date.now(),
        backpressure = false,
        firstQueueAt = null,
        firstConsecutiveQueueAt = null;
      // Native TCP receive window, then bounded real HTTP writes. No synthetic
      // Gateway frames or altered queue/send timeout in the API.
      while (sent < 1024 && samples.at(-1).rawSocketOpen) {
        const batch = await Promise.all(
          Array.from({ length: 8 }, async (_, i) => {
            const content = `E2E load ${sent + i} ` + "🟥".repeat(1900);
            const response = await api(
              f.owner,
              `/channels/${channel}/messages`,
              "POST",
              { content },
            );
            check(
              response.status === 201,
              "fixture-backpressure-load-write-failed",
              { status: response.status },
            );
            return Buffer.byteLength(content);
          }),
        );
        sent += batch.length;
        bytes += batch.reduce((a, b) => a + b, 0);
        lastCommit = Date.now();
        const sample = await runtime.processSample(raw.ready.port);
        backpressure ||= sample.rawSendQueueBytes > 0;
        if (sample.rawSendQueueBytes > 0) {
          firstQueueAt ??= sample.at;
          if (samples.at(-1).rawSendQueueBytes > 0)
            firstConsecutiveQueueAt ??= sample.at;
        }
        samples.push({ ...sample, messagesCommitted: sent });
      }
      check(backpressure, "fixture-tcp-backpressure-not-observed");
      const closed = await until(
        () => runtime.processSample(raw.ready.port),
        (s) => !s.rawSocketOpen,
        "slow-reader-not-disconnected-after-bounded-load",
        2_500,
      );
      const terminalMs = Date.now() - lastCommit;
      // This is a post-load observation budget, not a measurement from the
      // onset of a blocked application send. Source timeouts are separate.
      check(
        terminalMs <= 2_500,
        "slow-reader-post-load-observation-budget-exceeded",
        { controlConfirmed: true, terminalMs },
      );
      const marker = "E2E healthy client after raw TCP disconnect";
      check(
        (
          await api(f.owner, `/channels/${channel}/messages`, "POST", {
            content: marker,
          })
        ).status === 201,
        "fixture-healthy-client-write-failed",
      );
      const pane = f.member.page.getByRole("log", { name: "Nachrichten" });
      await until(
        async () => {
          // The real virtualized history can keep its scroll anchor during a
          // large burst. Move its viewport to the newest row without a reload.
          await pane.evaluate((el) => {
            el.scrollTop = el.scrollHeight;
          });
          return pane.getByText(marker, { exact: true }).isVisible();
        },
        Boolean,
        "healthy-app-client-no-progress-after-raw-disconnect",
        12_000,
      );
      return {
        rawReceiveBufferBytes: raw.ready.receiveBufferBytes,
        noReadsAfterPause: true,
        clientHeartbeatsContinued: true,
        messagesCommitted: sent,
        contentBytesCommitted: bytes,
        pendingTcpBytesWhileReceivePaused: true,
        blockedApplicationSendDirectlyMeasured: false,
        ownApiSocketDisconnected: true,
        terminalAfterLastCommitMs: terminalMs,
        pauseObservedAt,
        firstPendingQueueObservedAt: firstQueueAt,
        firstConsecutivePendingQueueSampleAt: firstConsecutiveQueueAt,
        lastCommitObservedAt: new Date(lastCommit).toISOString(),
        socketDisappearanceObservedAt: (
          samples.find((s) => !s.rawSocketOpen) ?? closed
        ).at,
        loadSampleGapsMs: samples
          .slice(1)
          .map((s, i) => Date.parse(s.at) - Date.parse(samples[i].at)),
        terminalPollingIntervalMs: 200,
        postLoadObservationBudgetMs: 2500,
        timeoutSourceEvidence: {
          apiSource: runtime.manifest.sourceSha,
          file: "api/src/gateway/conn.rs",
          sendMs: 1000,
          finalCloseMs: 1000,
          attribution: "source contract; not measured from blocked-send onset",
        },
        outboundQueueContractFrames: 128,
        healthyAppClientProgressAfterDisconnect: true,
        rssSamples: [
          samples[0],
          samples.reduce((a, b) => (a.rssKiB >= b.rssKiB ? a : b)),
          { ...closed, messagesCommitted: sent },
        ],
        rssClaim:
          "observed samples of the manifest-matched own API child under this load; no general memory bound",
      };
    } finally {
      await raw.close();
    }
  });
}
