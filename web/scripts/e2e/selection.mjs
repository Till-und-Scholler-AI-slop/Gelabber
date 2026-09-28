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
export function gate(results, selected, cleanup) {
  const absent = selected.filter((id) => !results.some((r) => r.id === id));
  const failures = results.filter(
    (r) =>
      ["FAIL", "BLOCKED"].includes(r.status) || r.fixtureRecovery === "BLOCKED",
  );
  const excluded = results
    .filter((r) => r.status === "NOT_RUN")
    .map((r) => r.id);
  return {
    passed:
      absent.length === 0 &&
      failures.length === 0 &&
      cleanup.every((r) => r.status === 204) &&
      results.some((r) => r.status === "PASS"),
    scope: selected.length ? "selected-cases" : "requested-suite",
    excluded,
    absent,
    // This runner cannot prove native picker, audible two-device, WAN or production gates.
    completeAcceptance: false,
  };
}
