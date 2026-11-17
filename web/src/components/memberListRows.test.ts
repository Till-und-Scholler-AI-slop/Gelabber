import { describe, expect, it } from "vitest";

import { memberListRows, memberRowEstimate } from "./memberListRows.ts";

const member = (user_id: string) => ({ user_id });

describe("member list rows", () => {
  it("puts each group's heading before its members", () => {
    const rows = memberListRows([
      { group: "o", label: "Online", members: [member("a"), member("b")] },
      { group: "x", label: "Offline", members: [member("c")] },
    ]);
    expect(
      rows.map((row) => (row.kind === "heading" ? row.label : row.key)),
    ).toEqual(["Online", "a", "b", "Offline", "c"]);
    expect(rows[0]).toMatchObject({ kind: "heading", count: 2, first: true });
    expect(rows[3]).toMatchObject({ kind: "heading", count: 1, first: false });
    expect(rows[4]).toMatchObject({ kind: "member", group: "x", position: 3 });
    // Positions count members across groups, headings left out.
    expect(
      rows.flatMap((row) => (row.kind === "member" ? [row.position] : [])),
    ).toEqual([1, 2, 3]);
  });

  it("keys rows apart from each other, headings from members", () => {
    const rows = memberListRows([
      { group: "o", label: "Online", members: [member("o")] },
      { group: "i", label: "Idle", members: [member("i")] },
    ]);
    const keys = rows.map((row) => row.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("is empty without members", () => {
    expect(memberListRows([])).toEqual([]);
  });

  it("guesses the first heading without the gap above it", () => {
    const [first, , second] = memberListRows([
      { group: "o", label: "Online", members: [member("a")] },
      { group: "x", label: "Offline", members: [member("b")] },
    ]);
    expect(memberRowEstimate(first)).toBeLessThan(memberRowEstimate(second));
    expect(memberRowEstimate(undefined)).toBe(0);
  });
});
