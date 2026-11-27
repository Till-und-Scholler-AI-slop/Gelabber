// The six role colours (v0.10, #516) are fixed tokens on :root, the same in
// every theme, so the dots and swatches of servers/roles.ts always resolve.
/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./index.css", import.meta.url), "utf8");

describe("role colour tokens", () => {
  it("defines --lr-role-* for each of the six colours", () => {
    for (const [color, hex] of Object.entries({
      amber: "#e0a04a",
      blue: "#6fb3d9",
      green: "#85d69b",
      pink: "#d98fb8",
      violet: "#b29ae6",
      coral: "#e08f6f",
    })) {
      expect(css).toContain(`--lr-role-${color}: ${hex};`);
    }
  });
});
