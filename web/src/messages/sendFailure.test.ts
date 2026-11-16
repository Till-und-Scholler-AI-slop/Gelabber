import { afterEach, describe, expect, it } from "vitest";

import { ApiError, type FieldErrors } from "../api/client.ts";
import { setLocale } from "../i18n/locale.ts";
import { sendFailureText } from "./sendFailure.ts";

const refused = (
  status: number,
  code: ConstructorParameters<typeof ApiError>[0],
  fields: FieldErrors = {},
  retryAfter: number | null = null,
) => new ApiError(code, status, "server text", fields, retryAfter);

describe("sendFailureText (#581)", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("says a GIF is gone instead of the generic bind text", () => {
    setLocale("en", { persist: false });
    const error = refused(422, "validation_failed", { gif: "invalid" });
    expect(sendFailureText(error, "bind", true)).toBe(
      "This GIF is no longer available.",
    );
    setLocale("de", { persist: false });
    expect(sendFailureText(error, "bind", true)).toBe(
      "Dieses GIF ist nicht mehr verfügbar.",
    );
  });

  it("says GIFs are off when the server has none", () => {
    setLocale("en", { persist: false });
    expect(
      sendFailureText(
        refused(422, "validation_failed", { gif: "unavailable" }),
        "bind",
        true,
      ),
    ).toBe("GIFs are turned off on this server.");
  });

  it("names the wait on a 429, for GIFs and for other sends", () => {
    setLocale("en", { persist: false });
    expect(
      sendFailureText(refused(429, "rate_limited", {}, 42), "bind", true),
    ).toBe("Too many GIFs. Try again in 42 seconds.");
    expect(
      sendFailureText(refused(429, "rate_limited", {}, 1), "bind", false),
    ).toBe("Too many messages. Try again in 1 second.");
    expect(
      sendFailureText(refused(429, "rate_limited"), "presign", false),
    ).toBe("Too many messages. Wait a moment.");
    expect(sendFailureText(refused(429, "rate_limited"), "bind", true)).toBe(
      "Too many GIFs. Wait a moment.",
    );
  });

  it("says sending is not allowed on a 403", () => {
    setLocale("en", { persist: false });
    expect(sendFailureText(refused(403, "forbidden"), "bind", false)).toBe(
      "You can't send messages here.",
    );
  });

  it("keeps the generic text with status and code for other refusals", () => {
    setLocale("en", { persist: false });
    expect(
      sendFailureText(
        refused(422, "validation_failed", { content: "too_long" }),
        "bind",
        false,
      ),
    ).toBe("Saving the message failed. (422 validation_failed)");
    expect(sendFailureText(refused(404, "not_found"), "presign", false)).toBe(
      "Preparing the upload failed. (404 not_found)",
    );
  });

  it("keeps the generic text without a server answer", () => {
    setLocale("en", { persist: false });
    expect(sendFailureText(new Error("offline"), "bind", true)).toBe(
      "Saving the message failed.",
    );
    expect(sendFailureText(null, "upload", false)).toBe("File upload failed.");
    expect(
      sendFailureText(refused(502, "upstream_unavailable"), "bind", true),
    ).toBe("Saving the message failed.");
  });
});
