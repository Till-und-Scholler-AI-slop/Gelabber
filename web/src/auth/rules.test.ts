import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";

import {
  OIDC_ERRORS,
  PASSWORD_MAX,
  apiErrorMessage,
  avatarFieldAfterRemoval,
  errorMessage,
  fieldMessage,
  fieldMessages,
  isOidcError,
  oidcErrorMessage,
  profileFormPatch,
  removesAvatarAtOnce,
  validateAvatarUrl,
  validateEmail,
  validateName,
  validateNewEmail,
  validatePassword,
} from "./rules.ts";

describe("validation rules (mirror of backend/api/src/auth/validate.rs)", () => {
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

  // Registration (new_email): one bare mailbox, as the server stores it.
  // Sign-in keeps the lenient check so older accounts stay reachable.
  it("a new email is one bare address", () => {
    expect(validateNewEmail(" Ada.Lovelace+chat@Example.org ")).toBeNull();
    expect(validateNewEmail("victim@bücher.de")).toBeNull();
    expect(validateNewEmail("o'neil_x-y@example.org")).toBeNull();
    for (const raw of [
      "a<victim@example.org>",
      "<victim@example.org>",
      '"victim"@example.org',
      '"a b"@example.org',
      "victim(comment)@example.org",
      "victim@ex%61mple.org",
      ".victim@example.org",
      "vic..tim@example.org",
      `${"a".repeat(65)}@example.org`,
    ]) {
      expect(validateNewEmail(raw), raw).toBe("invalid");
      expect(fieldMessage("email", "invalid")).toBe(
        "Das sieht nicht wie eine E-Mail-Adresse aus.",
      );
    }
    expect(validateEmail("a<victim@example.org>")).toBeNull();
    expect(validateNewEmail("")).toBe("required");
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

  it("avatar url accepts the API's own uploaded path, nothing else relative", () => {
    const uploaded =
      "/api/avatars/0b8d6a8e-3f1c-4f7e-9a51-2a1c3e4d5f60/7c9e6679-7425-40de-944b-e07fc1f90ae7";
    expect(validateAvatarUrl(uploaded)).toBeNull();
    expect(validateAvatarUrl("/api/avatars/x/y")).toBe("invalid");
    expect(validateAvatarUrl("/etc/passwd")).toBe("invalid");
    expect(validateAvatarUrl(`${uploaded}/../x`)).toBe("invalid");
  });
});

describe("copy", () => {
  it("has German text for every field code", () => {
    expect(fieldMessage("email", "required")).toBe("E-Mail-Adresse fehlt.");
    expect(fieldMessage("password", "too_short")).toBe("Mindestens 8 Zeichen.");
    expect(fieldMessage("email", "taken")).toMatch(/schon registriert/);
    expect(fieldMessage("avatar_url", "invalid")).toMatch(/https-URL/);
    expect(fieldMessage("current_password", "wrong")).toBe(
      "Das Passwort stimmt nicht.",
    );
    expect(fieldMessage("unknown_field", "invalid")).toBe(
      "Eingabe ist ungültig.",
    );
  });

  it("has copy for the role routes' field codes", () => {
    expect(fieldMessage("name", "taken")).toBe(
      "Diesen Namen hat schon eine Rolle.",
    );
    expect(fieldMessage("roles", "too_many")).toBe(
      "Ein Server hat höchstens 50 Rollen.",
    );
    expect(fieldMessage("role_ids", "invalid")).toMatch(/gibt es nicht mehr/);
    expect(fieldMessage("color", "invalid")).toBe("Farbe ist ungültig.");
    expect(fieldMessage("permissions", "invalid")).toBe("Rechte ist ungültig.");
    expect(fieldMessage("name", "too_long")).toBe("Name ist zu lang.");
  });

  // The server's bind turns down an image too large in pixels by its place.
  it("names an image with too many pixels", () => {
    expect(fieldMessage("attachment_ids.1", "too_long")).toBe(
      "Das Bild hat zu viele Pixel (max. 16384 px pro Seite, 64 Megapixel).",
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
    expect(errorMessage("insecure_transport")).toMatch(/https:\/\//);
  });
});

describe("profileFormPatch", () => {
  const uploaded =
    "/api/avatars/0b8d6a8e-3f1c-4f7e-9a51-2a1c3e4d5f60/7c9e6679-7425-40de-944b-e07fc1f90ae7";

  it("leaves an unchanged avatar out, so a stale form cannot undo an upload", () => {
    expect(profileFormPatch(" Ada ", uploaded, uploaded)).toEqual({
      name: "Ada",
    });
    expect(profileFormPatch("Ada", "", null)).toEqual({ name: "Ada" });
  });

  it("sends a changed or cleared avatar, trimmed", () => {
    expect(
      profileFormPatch("Ada", " https://cdn.example/a.png ", uploaded),
    ).toEqual({ name: "Ada", avatar_url: "https://cdn.example/a.png" });
    expect(profileFormPatch("Ada", "  ", "https://cdn.example/a.png")).toEqual({
      name: "Ada",
      avatar_url: "",
    });
  });
});

describe("avatar removal", () => {
  const uploaded =
    "/api/avatars/0b8d6a8e-3f1c-4f7e-9a51-2a1c3e4d5f60/7c9e6679-7425-40de-944b-e07fc1f90ae7";
  const link = "https://cdn.example/a.png";

  it("removes a saved upload at once", () => {
    expect(removesAvatarAtOnce(uploaded, uploaded)).toBe(true);
    expect(removesAvatarAtOnce(uploaded, ` ${uploaded} `)).toBe(true);
  });

  it("only clears the field for a link or an edited field", () => {
    expect(removesAvatarAtOnce(link, link)).toBe(false);
    expect(removesAvatarAtOnce(null, "")).toBe(false);
    // The field already holds something else: that edit is cleared, unsaved.
    expect(removesAvatarAtOnce(uploaded, link)).toBe(false);
  });

  it("keeps a link typed while the removal ran", () => {
    expect(avatarFieldAfterRemoval(uploaded)).toBe("");
    expect(avatarFieldAfterRemoval(link)).toBe(link);
    expect(avatarFieldAfterRemoval("https://cdn.exa")).toBe("https://cdn.exa");
    expect(avatarFieldAfterRemoval("")).toBe("");
  });
});

describe("copy in English", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("field and error messages follow the interface's language", () => {
    expect(fieldMessage("name", "required")).toBe("Name fehlt.");
    setLocale("en", { persist: false });
    expect(fieldMessage("email", "required")).toBe("Email address is missing.");
    expect(fieldMessage("password", "too_short")).toBe(
      "At least 8 characters.",
    );
    expect(fieldMessage("whatever", "invalid")).toBe("Input is invalid.");
    expect(errorMessage("invalid_credentials")).toBe(
      "Wrong email or password.",
    );
    expect(errorMessage("bad_request")).toBe(errorMessage("internal"));
  });
});

describe("admission and operator codes (#517)", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("has German and English text for the new form-level codes", () => {
    expect(errorMessage("invitation_required")).toBe(
      "Zum Registrieren brauchst du eine Einladung.",
    );
    expect(errorMessage("invitation_invalid")).toBe(
      "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt.",
    );
    expect(errorMessage("account_disabled")).toBe("Dieses Konto ist gesperrt.");
    expect(errorMessage("server_blocked")).toBe(
      "Dieser Server wurde vom Betreiber gesperrt.",
    );
    setLocale("en", { persist: false });
    expect(errorMessage("invitation_required")).toBe(
      "You need an invitation to register.",
    );
    expect(errorMessage("invitation_invalid")).toBe(
      "This invitation is unknown, expired or already used.",
    );
    expect(errorMessage("account_disabled")).toBe(
      "This account has been disabled.",
    );
    expect(errorMessage("server_blocked")).toBe(
      "This server has been blocked by the operator.",
    );
  });

  it("puts the API's invitation field codes at the invitation field", () => {
    expect(fieldMessages({ invitation: "required", email: "invalid" })).toEqual(
      {
        invitation: "Zum Registrieren brauchst du eine Einladung.",
        email: fieldMessage("email", "invalid"),
      },
    );
    expect(fieldMessage("invitation", "invalid")).toBe(
      "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt.",
    );
    expect(fieldMessage("invitation", "too_long")).toBe(
      "Einladungscode ist zu lang.",
    );
  });
});

describe("sign-in through a provider (#518)", () => {
  afterEach(() => setLocale("de", { persist: false }));

  const CODES = [
    "oidc_unavailable",
    "oidc_email_unverified",
    "oidc_account_unverified",
    "identity_taken",
    "password_required",
    "reauth_required",
    "oidc_wrong_origin",
    "session_changed",
  ] as const;

  it("has its own copy for every new error code, in both languages", () => {
    for (const locale of ["de", "en"] as const) {
      setLocale(locale, { persist: false });
      const texts = CODES.map((code) => errorMessage(code));
      expect(new Set(texts).size).toBe(CODES.length);
      for (const text of texts) expect(text).not.toBe(errorMessage("internal"));
    }
  });

  it("says why a provider sign-in came back", () => {
    for (const locale of ["de", "en"] as const) {
      setLocale(locale, { persist: false });
      for (const code of OIDC_ERRORS) {
        expect(oidcErrorMessage(code), code).not.toBe(errorMessage("internal"));
      }
    }
    setLocale("de", { persist: false });
    expect(oidcErrorMessage("cancelled")).toBe("Anmeldung abgebrochen.");
    expect(oidcErrorMessage("account_unverified")).toBe(
      errorMessage("oidc_account_unverified"),
    );
    expect(oidcErrorMessage("invitation_required")).toBe(
      errorMessage("invitation_required"),
    );
  });

  it("knows only the listed oidc_error codes", () => {
    expect(isOidcError("expired")).toBe(true);
    expect(isOidcError("registration_closed")).toBe(false);
    expect(isOidcError(42)).toBe(false);
  });
});

describe("directory codes (#520)", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("has German and English text for the join refusals", () => {
    expect(errorMessage("join_request_required")).toBe(
      "Diesem Server trittst du nur auf Anfrage bei.",
    );
    expect(errorMessage("join_request_declined")).toBe(
      "Deine Anfrage wurde nicht angenommen. Du kannst in ein paar Tagen wieder anfragen.",
    );
    expect(errorMessage("join_requests_full")).toBe(
      "Gerade sind zu viele Anfragen offen. Bitte später noch einmal versuchen.",
    );
    expect(errorMessage("already_member")).toBe(
      "Du bist schon Mitglied dieses Servers.",
    );
    setLocale("en", { persist: false });
    expect(errorMessage("join_request_required")).toBe(
      "This server can only be joined by request.",
    );
    expect(errorMessage("already_member")).toBe(
      "You are already a member of this server.",
    );
  });
  it("names the day a declined request may ask again", () => {
    const now = Date.UTC(2026, 10, 11, 12);
    const week = 7 * 24 * 3600;
    expect(
      apiErrorMessage({ code: "join_request_declined", retryAfter: week }, now),
    ).toBe(
      "Deine Anfrage wurde nicht angenommen. Du kannst ab dem 18. November 2026 wieder anfragen.",
    );
    expect(
      apiErrorMessage({ code: "join_request_declined", retryAfter: null }, now),
    ).toBe(errorMessage("join_request_declined"));
    setLocale("en", { persist: false });
    expect(
      apiErrorMessage({ code: "join_request_declined", retryAfter: week }, now),
    ).toBe(
      "Your request was not accepted. You can ask again from 18 November 2026.",
    );
    expect(
      apiErrorMessage({ code: "already_member", retryAfter: null }, now),
    ).toBe("You are already a member of this server.");
  });
});
