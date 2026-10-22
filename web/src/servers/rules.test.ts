import { describe, expect, it } from "vitest";

import {
  SERVER_NAME_MAX,
  slugifyChannelName,
  validateChannelName,
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
});
