import { describe, expect, it } from "vitest";

import {
  PASSWORD_MAX,
  errorMessage,
  fieldMessage,
  fieldMessages,
  validateAvatarUrl,
  validateEmail,
  validateName,
  validatePassword,
} from "./rules.ts";

describe("validation rules (mirror of api/src/auth/validate.rs)", () => {
  it("email", () => {
    expect(validateEmail(" Ada@Example.com ")).toBeNull();
    expect(validateEmail("")).toBe("required");
    expect(validateEmail("   ")).toBe("required");
    expect(validateEmail("nope")).toBe("invalid");
    expect(validateEmail("@example.com")).toBe("invalid");
    expect(validateEmail("ada@")).toBe("invalid");
    expect(validateEmail("ada@localhost")).toBe("invalid");
    expect(validateEmail("ada@@example.com")).toBe("invalid");
    expect(validateEmail("a da@example.com")).toBe("invalid");
    expect(validateEmail(`${"a".repeat(254)}@example.com`)).toBe("too_long");
  });

  it("password counts characters, not bytes", () => {
    expect(validatePassword("12345678")).toBeNull();
    expect(validatePassword("pässwörd")).toBeNull();
    expect(validatePassword("")).toBe("required");
    expect(validatePassword("1234567")).toBe("too_short");
    expect(validatePassword("x".repeat(PASSWORD_MAX + 1))).toBe("too_long");
  });

  it("name", () => {
    expect(validateName("  Ada ")).toBeNull();
    expect(validateName("   ")).toBe("required");
    expect(validateName("a\nb")).toBe("invalid");
    expect(validateName("x".repeat(65))).toBe("too_long");
  });

  it("avatar url accepts https only, or empty", () => {
    expect(validateAvatarUrl("")).toBeNull();
    expect(validateAvatarUrl("  ")).toBeNull();
    expect(validateAvatarUrl("https://cdn.example/a.png")).toBeNull();
    expect(validateAvatarUrl("HTTPS://cdn.example/a.png")).toBeNull();
    expect(validateAvatarUrl("javascript:alert(1)")).toBe("invalid");
    expect(validateAvatarUrl("ftp://x/y")).toBe("invalid");
    expect(validateAvatarUrl("https://")).toBe("invalid");
    expect(validateAvatarUrl("https:///etc")).toBe("invalid");
    expect(validateAvatarUrl("http://cdn.example/a.png")).toBe("invalid");
    expect(validateAvatarUrl("https://a b")).toBe("invalid");
  });
});

describe("copy", () => {
  it("has German text for every field code", () => {
    expect(fieldMessage("email", "required")).toBe("E-Mail-Adresse fehlt.");
    expect(fieldMessage("password", "too_short")).toBe("Mindestens 8 Zeichen.");
    expect(fieldMessage("email", "taken")).toMatch(/schon registriert/);
    expect(fieldMessage("avatar_url", "invalid")).toMatch(/https-URL/);
    expect(fieldMessage("unknown_field", "invalid")).toBe(
      "Eingabe ist ungültig.",
    );
  });

  it("maps server field codes to messages", () => {
    expect(fieldMessages({ email: "taken", name: "required" })).toEqual({
      email: "Diese E-Mail-Adresse ist schon registriert.",
      name: "Name fehlt.",
    });
  });

  it("has German text for every form-level code", () => {
    expect(errorMessage("invalid_credentials")).toBe(
      "E-Mail oder Passwort ist falsch.",
    );
    expect(errorMessage("timeout")).toMatch(/antwortet nicht/);
    expect(errorMessage("network")).toMatch(/Keine Verbindung/);
    expect(errorMessage("rate_limited")).toMatch(/Zu viele Anfragen/);
    expect(errorMessage("quota_exceeded")).toMatch(/Tageslimit/);
  });
});
