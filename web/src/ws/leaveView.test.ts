import { describe, expect, it } from "vitest";

import { shouldLeaveView } from "./leaveView.ts";

describe("shouldLeaveView", () => {
  it("leaves only when the current route is that server", () => {
    expect(shouldLeaveView("srv-a", "srv-a")).toBe(true);
    expect(shouldLeaveView("srv-b", "srv-a")).toBe(false);
    expect(shouldLeaveView(undefined, "srv-a")).toBe(false);
  });
});
