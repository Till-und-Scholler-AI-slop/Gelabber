/* global URL, process */
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("exact CI fault loop attempts both browsers and both profiles after first failure, retains nonzero, and omits SFU only for core", async () => {
  const workflow = await readFile(
    new URL("../../../.github/workflows/ci.yml", import.meta.url),
    "utf8",
  );
  const match =
    / {6}- name: Real app core and API\/Redis Live-lease faults in both browsers\n {8}run: \|\n([\s\S]*?)(?=\n {6}- name:)/.exec(
      workflow,
    );
  assert.ok(match, "Actual workflow loop required");
  const body = match[1]
    .split("\n")
    .map((line) => line.replace(/^ {10}/, ""))
    .join("\n");
  const dir = await mkdtemp(join(tmpdir(), "gelabber-ci-loop-control-"));
  try {
    const mockNode = `node() {
      printf '%s|%s|%s|%s\\n' "$GELABBER_E2E_BROWSER" "$GELABBER_E2E_PROFILE" "$GELABBER_E2E_MEDIA_FAULT_ENV" "$GELABBER_E2E_MEDIA_MANIFEST" >> "$E2E_LOOP_RECORD"
      if [ "$GELABBER_E2E_BROWSER" = chromium ] && [ "$GELABBER_E2E_PROFILE" = core-faults ]; then return 1; fi
      return 0
    }\n`;
    await assert.rejects(
      promisify(execFile)("bash", ["-c", mockNode + body], {
        env: {
          ...process.env,
          E2E_LOOP_RECORD: join(dir, "calls"),
          GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/private-media-control.env",
          GELABBER_E2E_MEDIA_MANIFEST: "/tmp/media-control-manifest.json",
        },
      }),
      (error) => error.code === 1 && error.stdout === "" && error.stderr === "",
    );
    const calls = (await readFile(join(dir, "calls"), "utf8"))
      .trim()
      .split("\n")
      .map((line) => line.split("|"));
    assert.deepEqual(calls, [
      ["chromium", "core-faults", "", ""],
      [
        "chromium",
        "media-faults",
        "/tmp/private-media-control.env",
        "/tmp/media-control-manifest.json",
      ],
      ["firefox", "core-faults", "", ""],
      [
        "firefox",
        "media-faults",
        "/tmp/private-media-control.env",
        "/tmp/media-control-manifest.json",
      ],
    ]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
