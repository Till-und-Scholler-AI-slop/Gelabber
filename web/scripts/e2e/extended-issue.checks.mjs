/* global URL, process */
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile } from "node:fs/promises";

// The step of e2e-extended.yml's `report` job, run with a fake `gh` that
// answers the issue search and records every other call.
async function report(result, openIssue) {
  const workflow = await readFile(
    new URL("../../../.github/workflows/e2e-extended.yml", import.meta.url),
    "utf8",
  );
  const match =
    / {6}- name: Open, update or close the issue\n[\s\S]*? {8}run: \|\n([\s\S]*?)(?=\n\S|\n {2}\S|$)/.exec(
      workflow,
    );
  assert.ok(match, "the report step is in the workflow");
  const body = match[1]
    .split("\n")
    .map((line) => line.replace(/^ {10}/, ""))
    .join("\n");
  const fakeGh = `gh() {
    if [ "$1 $2" = "issue list" ]; then printf '%s' "$OPEN_ISSUE"; return; fi
    printf '%s\\n' "$*" >&2
  }\n`;
  const { stderr } = await promisify(execFile)("bash", ["-c", fakeGh + body], {
    env: {
      ...process.env,
      OPEN_ISSUE: openIssue,
      RESULT: result,
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_REF_NAME: "v0.14",
      GITHUB_SHA: "abc123",
      RUN_URL: "https://example.test/run/1",
    },
  });
  return stderr.trim().split("\n").filter(Boolean);
}

test("a red push run opens the branch's issue, or comments on the open one", async () => {
  const opened = await report("failure", "");
  assert.equal(opened.length, 1);
  assert.match(
    opened[0],
    /^issue create --title Extended E2E fails on v0\.14 --body Run https:\/\/example\.test\/run\/1 failed at abc123/,
  );
  const commented = await report("failure", "7");
  assert.equal(commented.length, 1);
  assert.match(commented[0], /^issue comment 7 --body Run .* failed at abc123/);
});

test("a green push run closes the open issue with a comment, and does nothing without one", async () => {
  const closed = await report("success", "7");
  assert.deepEqual(closed, [
    "issue comment 7 --body Green again at abc123: https://example.test/run/1",
    "issue close 7",
  ]);
  assert.deepEqual(await report("success", ""), []);
});
