import { describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import { inviteCodeOf } from "./redirect.ts";
import {
  admissionCodes,
  isAdmissionError,
  showsInvitationField,
  showsInvitationOnly,
  validateInvitation,
  withoutInvitationField,
  type InvitationState,
} from "./registration.ts";

const base: InvitationState = {
  mode: "open",
  invitation: undefined,
  redirect: undefined,
  asked: false,
};

describe("the register page's invitation field", () => {
  it("stays away on an open instance", () => {
    expect(showsInvitationField(base)).toBe(false);
    expect(showsInvitationField({ ...base, mode: undefined })).toBe(false);
    expect(showsInvitationOnly(base)).toBe(false);
  });

  it("shows with a code in the address, whatever the mode", () => {
    for (const mode of ["open", "invitation", undefined] as const) {
      const state = { ...base, mode, invitation: "k7m2p9qrs4tx" };
      expect(showsInvitationField(state)).toBe(true);
      expect(showsInvitationOnly(state)).toBe(false);
    }
    expect(
      showsInvitationField({
        ...base,
        invitation: "k7m2p9qrs4tx",
        redirect: "/invite/abcdefghjkmn",
      }),
    ).toBe(true);
  });

  it("shows on an invitation-only instance, unless a server invite is followed", () => {
    const closed = { ...base, mode: "invitation" as const };
    expect(showsInvitationField(closed)).toBe(true);
    expect(showsInvitationOnly(closed)).toBe(true);
    const invited = { ...closed, redirect: "/invite/abcdefghjkmn" };
    expect(showsInvitationField(invited)).toBe(false);
    expect(showsInvitationOnly(invited)).toBe(false);
    // Any other target is no server invite.
    const elsewhere = { ...closed, redirect: "/s/1" };
    expect(showsInvitationField(elsewhere)).toBe(true);
    expect(showsInvitationOnly(elsewhere)).toBe(true);
  });

  it("comes up once the API asked for a code, mode check or not", () => {
    expect(
      showsInvitationField({ ...base, mode: undefined, asked: true }),
    ).toBe(true);
    expect(
      showsInvitationField({
        ...base,
        asked: true,
        redirect: "/invite/abcdefghjkmn",
      }),
    ).toBe(false);
  });
});

describe("the codes a registration sends", () => {
  it("sends the typed invitation, trimmed, and the followed server invite", () => {
    expect(admissionCodes(" k7m2p9qrs4tx ", undefined)).toEqual({
      invitation: "k7m2p9qrs4tx",
    });
    expect(admissionCodes(null, "/invite/abcdefghjkmn")).toEqual({
      server_invite: "abcdefghjkmn",
    });
    expect(admissionCodes("k7m2p9qrs4tx", "/invite/abcdefghjkmn?x=1")).toEqual({
      invitation: "k7m2p9qrs4tx",
      server_invite: "abcdefghjkmn",
    });
    expect(admissionCodes("  ", "/s/1")).toEqual({});
    expect(admissionCodes(null, undefined)).toEqual({});
  });

  it("reads the code of an invite target only", () => {
    expect(inviteCodeOf("/invite/abcdefghjkmn")).toBe("abcdefghjkmn");
    expect(inviteCodeOf("/invite/abcdefghjkmn/x")).toBe("abcdefghjkmn");
    expect(inviteCodeOf("/invite/")).toBeUndefined();
    expect(inviteCodeOf("/s/invite/abc")).toBeUndefined();
    expect(inviteCodeOf(undefined)).toBeUndefined();
  });

  it("asks for a code only where one is needed and none comes along", () => {
    expect(validateInvitation("", "invitation", undefined)).toBe("required");
    expect(validateInvitation("  ", "invitation", "/s/1")).toBe("required");
    expect(validateInvitation("x", "invitation", undefined)).toBeNull();
    expect(
      validateInvitation("", "invitation", "/invite/abcdefghjkmn"),
    ).toBeNull();
    expect(validateInvitation("", "open", undefined)).toBeNull();
    expect(validateInvitation("", undefined, undefined)).toBeNull();
  });
});

describe("admission refusals", () => {
  const required = new ApiError("invitation_required", 403, "x", {
    invitation: "required",
  });

  it("are the two invitation codes", () => {
    expect(isAdmissionError(required)).toBe(true);
    expect(
      isAdmissionError(
        new ApiError("invitation_invalid", 410, "x", { invitation: "invalid" }),
      ),
    ).toBe(true);
    expect(isAdmissionError(new ApiError("email_taken", 409, "x"))).toBe(false);
    expect(isAdmissionError(new Error("x"))).toBe(false);
  });

  it("lose their field where the page shows none", () => {
    const plain = withoutInvitationField(required);
    expect(plain.fields).toEqual({});
    expect(plain.code).toBe("invitation_required");
    expect(plain.status).toBe(403);
    expect(required.fields).toEqual({ invitation: "required" });
    const other = new ApiError("validation_failed", 422, "x", {
      email: "invalid",
    });
    expect(withoutInvitationField(other)).toBe(other);
  });
});
