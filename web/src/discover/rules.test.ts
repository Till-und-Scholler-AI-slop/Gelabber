import { describe, expect, it } from "vitest";

import {
  REPORT_DETAILS_MAX,
  REPORT_REASONS,
  isJoinMode,
  listingAction,
  listingMenuActions,
  validateDescription,
  validateLanguage,
  validateMessage,
  validateQuery,
  validateReportDetails,
} from "./rules.ts";

describe("directory rules", () => {
  it("requires a description only for a listed server", () => {
    expect(validateDescription("", true)).toBe("required");
    expect(validateDescription("   ", true)).toBe("required");
    expect(validateDescription("", false)).toBeNull();
    expect(validateDescription("  Filme am Freitag  ", true)).toBeNull();
  });

  it("keeps descriptions and messages to one line of 300 characters", () => {
    expect(validateDescription("x".repeat(300), true)).toBeNull();
    expect(validateDescription("x".repeat(301), true)).toBe("too_long");
    // Characters, not UTF-16 units.
    expect(validateDescription("🎬".repeat(300), true)).toBeNull();
    expect(validateDescription("eins\nzwei", true)).toBe("invalid");
    expect(validateDescription("tab\there", false)).toBe("invalid");
    expect(validateDescription("c1\u0085", false)).toBe("invalid");
    expect(validateMessage("")).toBeNull();
    expect(validateMessage("Hallo!")).toBeNull();
    expect(validateMessage("x".repeat(301))).toBe("too_long");
    expect(validateMessage("a\nb")).toBe("invalid");
  });

  it("takes searches of up to 100 characters", () => {
    expect(validateQuery("")).toBeNull();
    expect(validateQuery("rust")).toBeNull();
    expect(validateQuery(` ${"x".repeat(100)} `)).toBeNull();
    expect(validateQuery("x".repeat(101))).toBe("too_long");
    expect(validateQuery("a\u0000b")).toBe("invalid");
  });

  it("knows only the languages the directory names", () => {
    const languages = [{ code: "de" }, { code: "en" }];
    expect(validateLanguage("de", languages)).toBeNull();
    expect(validateLanguage("fr", languages)).toBe("invalid");
    expect(validateLanguage(null, languages)).toBe("required");
    expect(validateLanguage("", languages)).toBe("required");
  });

  it("knows the two join modes", () => {
    expect(isJoinMode("open")).toBe(true);
    expect(isJoinMode("request")).toBe(true);
    expect(isJoinMode("closed")).toBe(false);
  });

  it("picks the button a row offers", () => {
    const row = (
      member: boolean,
      join_mode: "open" | "request",
      request: "pending" | "declined" | null,
    ) => listingAction({ member, join_mode, request });
    expect(row(true, "request", null)).toBe("open");
    expect(row(true, "open", null)).toBe("open");
    expect(row(false, "open", null)).toBe("join");
    expect(row(false, "open", "pending")).toBe("join");
    expect(row(false, "request", null)).toBe("request");
    expect(row(false, "request", "pending")).toBe("requested");
    expect(row(false, "request", "declined")).toBe("declined");
  });

  it("offers to withdraw only a waiting request, and a report always", () => {
    expect(listingMenuActions({ request: null })).toEqual(["report"]);
    expect(listingMenuActions({ request: "declined" })).toEqual(["report"]);
    expect(listingMenuActions({ request: "pending" })).toEqual([
      "report",
      "withdraw",
    ]);
  });

  it("checks a report's details like operator's route", () => {
    expect(REPORT_REASONS).toEqual([
      "spam",
      "harassment",
      "sexual",
      "illegal",
      "other",
    ]);
    expect(validateReportDetails("", "spam")).toBeNull();
    expect(validateReportDetails("", "other")).toBe("required");
    expect(validateReportDetails("   ", "other")).toBe("required");
    expect(
      validateReportDetails("Zeile eins\nZeile zwei\t", "other"),
    ).toBeNull();
    expect(validateReportDetails("a\u0000b", "spam")).toBe("invalid");
    expect(
      validateReportDetails("ä".repeat(REPORT_DETAILS_MAX), "spam"),
    ).toBeNull();
    expect(
      validateReportDetails("x".repeat(REPORT_DETAILS_MAX + 1), "other"),
    ).toBe("too_long");
  });
});
