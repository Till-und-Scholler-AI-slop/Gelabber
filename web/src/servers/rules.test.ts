import { describe, expect, it } from "vitest";

import {
  ROLE_NAME_MAX,
  validateRoleName,
  SERVER_NAME_MAX,
  slugifyChannelName,
  validateChannelName,
  validateName,
} from "./rules.ts";

describe("server rules (mirror of backend/api/src/servers/validate.rs)", () => {
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

describe("role names (validate::role_name)", () => {
  it("trims and bounds the length", () => {
    expect(validateRoleName("  Moderation ")).toBeNull();
    expect(validateRoleName("   ")).toBe("required");
    expect(validateRoleName("x".repeat(ROLE_NAME_MAX))).toBeNull();
    expect(validateRoleName("x".repeat(ROLE_NAME_MAX + 1))).toBe("too_long");
  });

  it("refuses control characters, C1 included", () => {
    expect(validateRoleName("Mod\u0007")).toBe("invalid");
    expect(validateRoleName("Mod\u0085s")).toBe("invalid");
  });

  it("refuses the fixed roles' names and other roles' names, ignoring case", () => {
    expect(validateRoleName("alle mitglieder")).toBe("taken");
    expect(validateRoleName("OWNER")).toBe("taken");
    expect(validateRoleName("Inhaber")).toBe("taken");
    expect(validateRoleName("All Members")).toBe("taken");
    expect(validateRoleName("stammgäste", ["Stammgäste"])).toBe("taken");
    expect(validateRoleName("Stammgäste", ["Moderation"])).toBeNull();
  });
});
