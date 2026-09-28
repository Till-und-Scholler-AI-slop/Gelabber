/* global URL, fetch */
import { readFile } from "node:fs/promises";
import { api, check, navigate, until } from "./harness.mjs";

export async function storageFaultScenarios(h, f, runtime) {
  const id = "storage-delete-outage-durable-cleanup";
  if (!runtime) {
    h.blocked(
      id,
      "requires owned API process, dedicated DB/bucket and fault proxies",
      ["10"],
    );
    h.blocked(
      "upload-abandoned-object-cleanup",
      "requires owned API, dedicated DB/bucket and own expiry fixtures",
      ["10"],
    );
    return;
  }
  await h.run(id, ["10"], async () => {
    await navigate(f.owner, f.textPath, f.base);
    const channel = f.textPath.split("/").at(-1);
    const bytes = await readFile(
      new URL("../fixtures/smoke.png", import.meta.url),
    );
    let uploadUrl, attachmentId, messageId;
    const statuses = [];
    const onResponse = async (response) => {
      const path = new URL(response.url()).pathname;
      if (
        response.request().method() === "POST" &&
        path === `/api/channels/${channel}/attachments`
      ) {
        const p = await response.json();
        uploadUrl = p.upload_url;
        attachmentId = p.id;
      }
      if (
        response.request().method() === "POST" &&
        path === `/api/channels/${channel}/messages`
      ) {
        statuses.push(response.status());
        if (response.status() === 201) messageId = (await response.json()).id;
      }
    };
    f.owner.page.on("response", onResponse);
    try {
      await f.owner.page.locator('input[type="file"]').setInputFiles({
        name: "durable.png",
        mimeType: "image/png",
        buffer: bytes,
      });
      await f.owner.page
        .locator("form textarea")
        .last()
        .fill("E2E durable storage control");
      await f.owner.page
        .getByRole("button", { name: "Senden", exact: true })
        .click();
      await until(
        async () => ({
          ready: Boolean(uploadUrl && attachmentId && messageId),
        }),
        (r) => r.ready,
        "fixture-upload-bind-not-completed",
      );
      const redirect = await f.member.context.request.get(
        `${f.base}/api/attachments/${attachmentId}`,
        { maxRedirects: 0 },
      );
      check(
        [302, 307].includes(redirect.status()),
        "fixture-download-redirect-missing",
        { status: redirect.status() },
      );
      const signedGet = redirect.headers().location; // private, retained only in memory
      const before = await f.member.context.request.get(signedGet);
      check(
        before.status() === 200 && (await before.body()).equals(bytes),
        "fixture-object-byte-control-failed",
      );
      const key = await runtime.sql(
        `SELECT object_key FROM attachments WHERE id='${attachmentId}'::uuid`,
      );
      check(key.length > 0, "fixture-object-key-missing");
      const routeControl = await api(
        f.owner,
        `/messages/${messageId}`,
        "PATCH",
        { content: "E2E durable storage control" },
      );
      check(
        routeControl.status === 200,
        "fixture-message-item-route-control-failed",
        { status: routeControl.status },
      );
      const session = await api(f.owner, "/auth/session");
      runtime.database.block();
      let dbStatus;
      try {
        dbStatus = await f.owner.page.evaluate(
          async ({ messageId, csrf }) =>
            (
              await fetch(`/api/messages/${messageId}`, {
                method: "DELETE",
                headers: { "X-CSRF-Token": csrf },
              })
            ).status,
          { messageId, csrf: session.body.csrf_token },
        );
        check(dbStatus >= 500, "db-outage-delete-falsely-succeeded", {
          status: dbStatus,
          controlConfirmed: true,
        });
        check(
          (await runtime.sql(
            `SELECT count(*) FROM messages WHERE id='${messageId}'::uuid`,
          )) === "1" &&
            (await runtime.sql(
              `SELECT count(*) FROM attachments WHERE id='${attachmentId}'::uuid`,
            )) === "1",
          "db-failed-delete-partially-committed",
          { controlConfirmed: true },
        );
        const stillThere = await f.member.context.request.get(signedGet);
        check(
          stillThere.status() === 200 &&
            (await stillThere.body()).equals(bytes),
          "db-failed-delete-removed-existing-blob",
          { controlConfirmed: true },
        );
      } finally {
        runtime.database.restore();
      }
      await until(
        async () => (await api(f.owner, "/auth/session")).status,
        (s) => s === 200,
        "fixture-db-recovery-deadline",
        15_000,
      );
      runtime.storage.block("DELETE");
      try {
        const removed = await api(f.owner, `/messages/${messageId}`, "DELETE");
        check(
          removed.status === 204,
          "delete-did-not-commit-during-minio-outage",
          { status: removed.status, controlConfirmed: true },
        );
        await until(
          async () => ({
            failures: runtime.storage.counters.deleteFailures,
            jobs: Number(
              await runtime.sql(
                `SELECT count(*) FROM storage_cleanup WHERE object_key='${key.replaceAll("'", "''")}'`,
              ),
            ),
          }),
          (s) => s.failures > 0 && s.jobs === 1,
          "cleanup-intent-lost-during-storage-outage",
          15_000,
        );
        check(
          (await runtime.sql(
            `SELECT count(*) FROM messages WHERE id='${messageId}'::uuid`,
          )) === "0" &&
            (await runtime.sql(
              `SELECT count(*) FROM attachments WHERE id='${attachmentId}'::uuid`,
            )) === "0",
          "committed-delete-left-metadata",
        );
        check(
          (await f.member.context.request.get(signedGet)).status() === 200,
          "fixture-minio-delete-fault-not-effective",
        );
      } finally {
        runtime.storage.restore();
      }
      await until(
        async () => (await f.member.context.request.get(signedGet)).status(),
        (s) => s === 404,
        "cleanup-did-not-delete-after-storage-restored",
        20_000,
      );
      // Exact original PUT may still be valid. The durable intent must delete its late replay.
      const replay = await f.owner.context.request.put(uploadUrl, {
        data: bytes,
        headers: { "Content-Type": "image/png" },
      });
      check(replay.status() === 200, "fixture-original-put-replay-not-valid", {
        status: replay.status(),
      });
      check(
        (await f.member.context.request.get(signedGet)).status() === 200,
        "fixture-replayed-object-not-observed",
      );
      await until(
        async () => (await f.member.context.request.get(signedGet)).status(),
        (s) => s === 404,
        "late-put-replay-escaped-durable-cleanup",
        20_000,
      );
      return {
        browserUploadBind: statuses,
        originalDownloadByteEqual: true,
        dbFaultDeleteStatus: dbStatus,
        dbFailurePreservedMessageAndBlob: true,
        storageDeleteFaultStatus: 503,
        deleteFailures: runtime.storage.counters.deleteFailures,
        durableIntentDuringOutage: true,
        deleteAfterRestore: true,
        originalPutReplayStatus: replay.status(),
        replayDeleted: true,
      };
    } finally {
      runtime.database.restore();
      runtime.storage.restore();
      f.owner.page.off("response", onResponse);
    }
  });
  const cleanupId = "upload-abandoned-object-cleanup";
  if (!runtime) {
    h.blocked(
      cleanupId,
      "requires owned API, dedicated DB/bucket and accelerated own fixture deadlines",
      ["10"],
    );
    return;
  }
  await h.run(cleanupId, ["10"], async () => {
    const channel = f.textPath.split("/").at(-1);
    const bytes = await readFile(
      new URL("../fixtures/smoke.png", import.meta.url),
    );
    const controls = [];
    for (const uploaded of [false, true]) {
      const p = await api(f.owner, `/channels/${channel}/attachments`, "POST", {
        filename: "abandoned.png",
        content_type: "image/png",
        size: bytes.length,
      });
      check(p.status === 201, "fixture-abandoned-presign-failed", {
        status: p.status,
      });
      if (uploaded) {
        const put = await f.owner.context.request.put(p.body.upload_url, {
          data: bytes,
          headers: p.body.headers,
        });
        check(put.status() === 200, "fixture-abandoned-put-failed", {
          status: put.status(),
        });
      }
      const object = await runtime.attachmentObject(p.body.id);
      check(
        (await object.status()) === (uploaded ? 200 : 404),
        "fixture-abandoned-object-control-failed",
      );
      const reserved = Number(
        await runtime.sql(
          `SELECT reserved FROM upload_daily_usage WHERE uploader_id='${f.owner.id}'::uuid AND day=CURRENT_DATE`,
        ),
      );
      const consumed = Number(
        await runtime.sql(
          `SELECT consumed FROM upload_daily_usage WHERE uploader_id='${f.owner.id}'::uuid AND day=CURRENT_DATE`,
        ),
      );
      runtime.storage.block("HEAD");
      const failures = runtime.storage.counters.headFailures;
      try {
        await runtime.sql(
          `UPDATE attachments SET expires_at=clock_timestamp()-interval '16 minutes',expiry_retry_at=clock_timestamp() WHERE id='${p.body.id}'::uuid`,
        );
        await until(
          async () => runtime.storage.counters.headFailures,
          (n) => n > failures,
          "fixture-expiry-head-fault-not-exercised",
          10_000,
        );
        check(
          (await runtime.sql(
            `SELECT count(*) FROM attachments WHERE id='${p.body.id}'::uuid`,
          )) === "1" &&
            Number(
              await runtime.sql(
                `SELECT reserved FROM upload_daily_usage WHERE uploader_id='${f.owner.id}'::uuid AND day=CURRENT_DATE`,
              ),
            ) === reserved,
          "head-outage-refunded-or-lost-reservation",
          { controlConfirmed: true },
        );
      } finally {
        runtime.storage.restore();
      }
      // Advance only this owned row's retry clock, avoiding the production one-minute backoff.
      await runtime.sql(
        `UPDATE attachments SET expiry_retry_at=clock_timestamp() WHERE id='${p.body.id}'::uuid`,
      );
      await until(
        async () =>
          Number(
            await runtime.sql(
              `SELECT count(*) FROM attachments WHERE id='${p.body.id}'::uuid`,
            ),
          ),
        (n) => n === 0,
        "abandoned-upload-metadata-not-cleaned",
        15_000,
      );
      await until(
        () => object.status(),
        (status) => status === 404,
        "abandoned-upload-object-not-cleaned",
        15_000,
      );
      const reservedAfter = Number(
        await runtime.sql(
          `SELECT reserved FROM upload_daily_usage WHERE uploader_id='${f.owner.id}'::uuid AND day=CURRENT_DATE`,
        ),
      );
      const consumedAfter = Number(
        await runtime.sql(
          `SELECT consumed FROM upload_daily_usage WHERE uploader_id='${f.owner.id}'::uuid AND day=CURRENT_DATE`,
        ),
      );
      check(
        reservedAfter === reserved - bytes.length &&
          consumedAfter === consumed + (uploaded ? bytes.length : 0),
        "abandoned-upload-quota-transition-wrong",
        { controlConfirmed: true, uploaded },
      );
      controls.push({
        uploaded,
        actualObjectBefore: uploaded ? 200 : 404,
        headFaultStatus: 503,
        reservationRetainedDuringFault: true,
        metadataRemoved: true,
        objectAfter: 404,
        reservedBytesReleased: bytes.length,
        consumedBytesAdded: uploaded ? bytes.length : 0,
      });
    }
    return {
      controls,
      acceleratedClock:
        "only owned attachment expiry/grace and retry timestamps; production deadlines unchanged",
    };
  });
}
