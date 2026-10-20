import { describe, expect, it } from "vitest";

import { centeredCrop } from "./avatarImage.ts";
import {
  BIO_MAX,
  PRONOUNS_MAX,
  STATUS_TEXT_MAX,
  cardFormOf,
  fieldMessage,
  isUploadedAvatar,
  isUploadedBanner,
  profileCardPatch,
  profileTextLength,
  safeColor,
  validateBio,
  validateCardForm,
  validateColor,
  validatePronouns,
  validateStatusText,
} from "./rules.ts";
import { bannerOf } from "../components/profileStyle.ts";

const ID = "0f8fad5b-d9cb-469f-a165-70867728950e";

describe("profile card rules (mirror of api/src/auth/validate.rs)", () => {
  it("texts: empty clears, limits in characters, control characters", () => {
    expect(validateBio("")).toBeNull();
    expect(validateBio("  Hallo\r\nWelt  ")).toBeNull();
    expect(validateBio("ä".repeat(BIO_MAX))).toBeNull();
    expect(validateBio("x".repeat(BIO_MAX + 1))).toBe("too_long");
    expect(validateBio("a\tb")).toBe("invalid");
    expect(validateBio("a\u0085b")).toBe("invalid");
    for (const bidi of ["\u202a", "\u202e", "\u2066", "\u2069"]) {
      expect(validateStatusText(`a${bidi}b`)).toBe("invalid");
      expect(validatePronouns(`a${bidi}b`)).toBe("invalid");
      expect(validateBio(`a${bidi}b`)).toBe("invalid");
    }
    // Marks, ZWJ emoji and zero-width spaces stay allowed.
    expect(validateStatusText("\u05e9\u200f!")).toBeNull();
    expect(validateStatusText("👩\u200d💻")).toBeNull();
    expect(validateStatusText("a\u200bb")).toBeNull();
    expect(validatePronouns("sie/ihr")).toBeNull();
    expect(validatePronouns("x".repeat(PRONOUNS_MAX + 1))).toBe("too_long");
    expect(validateStatusText("zwei\nZeilen")).toBe("invalid");
    expect(validateStatusText("🦀".repeat(STATUS_TEXT_MAX))).toBeNull();
    expect(validateStatusText("x".repeat(STATUS_TEXT_MAX + 1))).toBe(
      "too_long",
    );
    expect(profileTextLength(" a\r\nb ")).toBe(3);
  });

  it("colors are #rrggbb or unset", () => {
    expect(validateColor("")).toBeNull();
    expect(validateColor("#A1b2C3")).toBeNull();
    for (const raw of ["a1b2c3", "#abc", "#a1b2c3d4", "red", "url(x)"]) {
      expect(validateColor(raw)).toBe("invalid");
    }
    expect(safeColor("#00aa88")).toBe("#00aa88");
    expect(safeColor("red; background: url(x)")).toBeNull();
    expect(safeColor(null)).toBeNull();
  });

  it("uploaded banners are told apart from avatars", () => {
    expect(isUploadedBanner(`/api/banners/${ID}/${ID}`)).toBe(true);
    expect(isUploadedBanner(`/api/avatars/${ID}/${ID}`)).toBe(false);
    expect(isUploadedAvatar(`/api/banners/${ID}/${ID}`)).toBe(false);
    expect(isUploadedBanner("https://cdn.example/banner.png")).toBe(false);
  });

  it("the PATCH carries only changed fields, normalised like the API", () => {
    const saved = {
      bio: "Hallo",
      pronouns: "sie/ihr",
      accent_color: "#00aa88",
      avatar_decoration: "ring" as const,
    };
    const form = cardFormOf(saved);
    expect(profileCardPatch(form, saved)).toEqual({});
    expect(
      profileCardPatch(
        {
          ...form,
          pronouns: " sie/ihr ",
          accent_color: "#00AA88",
          status_text: " Da ",
          avatar_decoration: "",
        },
        saved,
      ),
    ).toEqual({ status_text: "Da", avatar_decoration: "" });
    expect(profileCardPatch({ ...form, bio: "" }, saved)).toEqual({ bio: "" });
  });

  it("validates the whole card form with German messages", () => {
    const codes = validateCardForm({
      ...cardFormOf({}),
      bio: "x".repeat(BIO_MAX + 1),
      banner_color: "blau",
    });
    expect(codes).toEqual({ bio: "too_long", banner_color: "invalid" });
    expect(fieldMessage("bio", "too_long")).toBe("Über mich ist zu lang.");
    expect(fieldMessage("banner_color", "invalid")).toContain("#rrggbb");
    expect(fieldMessage("banner_url", "stale")).toContain("Lade die Seite neu");
  });
});

describe("banner", () => {
  it("prefers the image, then the banner color, then the accent", () => {
    const image = `/api/banners/${ID}/${ID}`;
    expect(bannerOf({ banner_url: image, banner_color: "#111111" })).toEqual({
      image,
    });
    expect(
      bannerOf({ banner_color: "#111111", accent_color: "#222222" }),
    ).toEqual({ color: "#111111" });
    expect(bannerOf({ accent_color: "#222222" })).toEqual({
      color: "#222222",
    });
    expect(bannerOf({ banner_color: "not-a-color" })).toBeNull();
    // Anything but our own upload path is ignored, like a bad color.
    for (const banner_url of [
      "https://cdn.example/banner.png",
      "/api/banners/../x",
      `/api/avatars/${ID}/${ID}`,
      "javascript:alert(1)",
    ]) {
      expect(bannerOf({ banner_url, banner_color: "#111111" })).toEqual({
        color: "#111111",
      });
    }
    expect(bannerOf({})).toBeNull();
  });

  it("crops the centered strip with the target ratio", () => {
    expect(centeredCrop(1200, 400, 960, 320)).toEqual({
      x: 0,
      y: 0,
      width: 1200,
      height: 400,
    });
    // Tall photo: full width, a centered 3:1 band.
    expect(centeredCrop(900, 1200, 960, 320)).toEqual({
      x: 0,
      y: 450,
      width: 900,
      height: 300,
    });
    // Very wide panorama: full height, centered.
    expect(centeredCrop(3000, 300, 960, 320)).toEqual({
      x: 1050,
      y: 0,
      width: 900,
      height: 300,
    });
    // Square avatar crop is unchanged by the generalisation.
    expect(centeredCrop(400, 300, 256, 256)).toEqual({
      x: 50,
      y: 0,
      width: 300,
      height: 300,
    });
  });
});
