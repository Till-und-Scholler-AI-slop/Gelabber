import { describe, expect, it } from "vitest";

import {
  SERVER_NAME_MAX,
  slugifyChannelName,
  validateChannelName,
  validateExpiresInHours,
  validateMaxUses,
  validateName,
} from "./rules.ts";

describe("server rules (mirror of api/src/servers/validate.rs)", () => {
  it("validates names like the API", () => {
    expect(validateName("  Team ")).toBeNull();
    expect(validateName("   ")).toBe("required");
    expect(validateName("x".repeat(SERVER_NAME_MAX + 1))).toBe("too_long");
    expect(validateName("a\tb")).toBe("invalid");
  });

  it("slugifies text channel names exactly like the API", () => {
    expect(slugifyChannelName("General")).toBe("general");
    expect(slugifyChannelName("  Off Topic ")).toBe("off-topic");
    expect(slugifyChannelName("#dev__chat")).toBe("dev-chat");
    expect(slugifyChannelName("--a--b--")).toBe("a-b");
    expect(slugifyChannelName("Ünïcode Ok")).toBe("ünïcode-ok");
    expect(slugifyChannelName("###")).toBe("");
  });

  it("rejects text channel names that slugify to nothing, keeps voice names", () => {
    expect(validateChannelName("###", "text")).toBe("invalid");
    expect(validateChannelName("###", "voice")).toBeNull();
    expect(validateChannelName("", "voice")).toBe("required");
  });

  it("checks invite limits, empty meaning unlimited", () => {
    expect(validateMaxUses("")).toBeNull();
    expect(validateMaxUses("5")).toBeNull();
    expect(validateMaxUses("0")).toBe("invalid");
    expect(validateMaxUses("1.5")).toBe("invalid");
    expect(validateMaxUses("10001")).toBe("invalid");
    expect(validateExpiresInHours("")).toBeNull();
    expect(validateExpiresInHours("24")).toBeNull();
    expect(validateExpiresInHours("-1")).toBe("invalid");
    expect(validateExpiresInHours("abc")).toBe("invalid");
  });
});
