import assert from "node:assert/strict";

export function selection(value = "") {
  const ids = value.split(",").filter(Boolean);
  assert.ok(
    ids.every((id) => /^[a-z0-9-]+$/.test(id)),
    "Invalid E2E case ID",
  );
  assert.equal(new Set(ids).size, ids.length, "Duplicate selected E2E case");
  return ids;
}
export function wanted(ids, id) {
  return ids.length === 0 || ids.includes(id);
}
export function requiredCase(ids, id, setup = false) {
  return setup || wanted(ids, id);
}
/** Whether a FAIL row is the failure its entry expects: the same reason,
 * and the same file:line when the entry pins a `site`. */
function expectedFailure(row, entry) {
  return (
    row.status === "FAIL" &&
    !!entry &&
    row.reason === entry.reason &&
    (!entry.site ||
      row.site === entry.site ||
      !!row.site?.startsWith(`${entry.site}:`))
  );
}
/**
 * `expected`: case id -> { issue, reason, site? }, for scenarios known to
 * fail (a profile's `expectedFailures`). A FAIL with that reason (and site)
 * leaves the gate green; a FAIL for another reason, a PASS and a BLOCKED
 * turn it red. A PASS stays red until the entry is removed.
 */
export function gate(results, selected, cleanup, expected = {}) {
  const absent = selected.filter((id) => !results.some((r) => r.id === id));
  const expectedFailures = results
    .filter((r) => expectedFailure(r, expected[r.id]))
    .map((r) => ({ id: r.id, issue: expected[r.id].issue, reason: r.reason }));
  // Listed, but failed otherwise than its entry says.
  const otherFailures = results
    .filter(
      (r) =>
        r.status === "FAIL" &&
        expected[r.id] &&
        !expectedFailure(r, expected[r.id]),
    )
    .map((r) => ({
      id: r.id,
      issue: expected[r.id].issue,
      reason: r.reason,
      site: r.site,
    }));
  const nowPassing = results
    .filter((r) => r.status === "PASS" && expected[r.id])
    .map((r) => ({ id: r.id, issue: expected[r.id].issue }));
  const failures = results.filter(
    (r) =>
      (r.status === "FAIL" && !expectedFailure(r, expected[r.id])) ||
      r.status === "BLOCKED" ||
      r.fixtureRecovery === "BLOCKED",
  );
  const excluded = results
    .filter((r) => r.status === "NOT_RUN")
    .map((r) => r.id);
  return {
    passed:
      absent.length === 0 &&
      failures.length === 0 &&
      nowPassing.length === 0 &&
      cleanup.every((r) => r.status === 204) &&
      results.some((r) => r.status === "PASS"),
    scope: selected.length ? "selected-cases" : "requested-suite",
    excluded,
    absent,
    expectedFailures,
    otherFailures,
    // Listed as failing but passed: remove them from the profile's list.
    nowPassing,
    // This runner cannot prove native picker, audible two-device, WAN or production gates.
    completeAcceptance: false,
  };
}
