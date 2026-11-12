import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";

const completeOidc = vi.hoisted(() => vi.fn());
vi.mock("./session.ts", () => ({ completeOidc }));

import { completeNameStep, isDeadToken } from "./nameStep.ts";

const TOKEN = "ab".repeat(32);
const signedIn = (redirect: string | null) => ({
  user: { id: "u1" },
  csrf_token: "c",
  redirect,
});

/** The API refuses `complete` with `error`. */
const refuse = (error: ApiError) =>
  completeOidc.mockImplementation(() => Promise.reject(error));

beforeEach(() => {
  completeOidc.mockReset();
});

describe("Konto anlegen on the name step", () => {
  it("creates the account with the trimmed name and goes on to its target", async () => {
    completeOidc.mockResolvedValue(signedIn("/invite/abc123"));
    expect(await completeNameStep(TOKEN, "  Lena  ")).toEqual({
      kind: "signed_in",
      href: "/invite/abc123",
    });
    expect(completeOidc).toHaveBeenCalledWith(TOKEN, "Lena");
  });

  it("goes to the start without a target, or with an unsafe one", async () => {
    completeOidc.mockResolvedValue(signedIn(null));
    expect(await completeNameStep(TOKEN, "Lena")).toEqual({
      kind: "signed_in",
      href: "/",
    });
    completeOidc.mockResolvedValue(signedIn("//evil.example"));
    expect(await completeNameStep(TOKEN, "Lena")).toEqual({
      kind: "signed_in",
      href: "/",
    });
  });

  it("turns into the dead link when the step is gone", async () => {
    refuse(new ApiError("token_invalid", 410, "gone"));
    expect(await completeNameStep(TOKEN, "Lena")).toEqual({ kind: "dead" });
  });

  it("shows an admission refusal as the form's error, without a field", async () => {
    refuse(
      new ApiError("invitation_required", 403, "no", {
        invitation: "required",
      }),
    );
    const outcome = await completeNameStep(TOKEN, "Lena");
    expect(outcome.kind).toBe("refused");
    const error = (outcome as { error: ApiError }).error;
    expect(error.code).toBe("invitation_required");
    expect(error.fields).toEqual({});
  });

  it("passes other refusals on unchanged", async () => {
    for (const refusal of [
      new ApiError("account_disabled", 403, "disabled"),
      new ApiError("validation_failed", 422, "bad", { name: "too_long" }),
    ]) {
      refuse(refusal);
      expect(await completeNameStep(TOKEN, "Lena")).toEqual({
        kind: "refused",
        error: refusal,
      });
    }
  });

  it("knows a dead token only by token_invalid", () => {
    expect(isDeadToken(new ApiError("token_invalid", 410, ""))).toBe(true);
    expect(isDeadToken(new ApiError("not_found", 404, ""))).toBe(false);
    expect(isDeadToken(new Error("token_invalid"))).toBe(false);
  });
});
