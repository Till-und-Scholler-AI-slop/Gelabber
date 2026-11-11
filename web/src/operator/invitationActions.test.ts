import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  confirm: vi.fn<
    (request: { title: string; tone?: string }) => Promise<boolean>
  >(async () => true),
  notify: vi.fn(),
  notifyError: vi.fn(),
  copyText: vi.fn<(text: string) => Promise<boolean>>(async () => true),
  api: {
    revokeInvitation: vi.fn<(id: string) => Promise<unknown>>(async () => null),
  },
}));
vi.mock("../components/confirm.ts", () => ({ confirmAction: fixture.confirm }));
vi.mock("../components/toasts.ts", () => ({
  notify: fixture.notify,
  notifyError: fixture.notifyError,
}));
vi.mock("../servers/clipboard.ts", () => ({ copyText: fixture.copyText }));
vi.mock("./api.ts", () => fixture.api);

import {
  EMPTY_INVITATION_FORM,
  canSendMail,
  checkInvitationForm,
  copyInvitationLink,
  invitationFieldMessages,
  invitationInput,
  invitationMenuEntries,
  revokeInvitation,
} from "./invitationActions.ts";

const labels = (entries: MenuEntry[]) =>
  entries.map((entry) => (entry.kind === "item" ? entry.label : entry.kind));

beforeEach(() => {
  vi.clearAllMocks();
  fixture.confirm.mockImplementation(async () => true);
  fixture.copyText.mockImplementation(async () => true);
});
afterEach(() => setLocale("de", { persist: false }));

describe("invitation form", () => {
  it("sends only what was entered, seven days by default", () => {
    expect(invitationInput(EMPTY_INVITATION_FORM, true)).toEqual({
      expires_in_days: 7,
    });
    expect(
      invitationInput(
        {
          email: " ada@example.test ",
          note: " Filmclub ",
          expiry: "never",
          sendMail: true,
        },
        true,
      ),
    ).toEqual({
      email: "ada@example.test",
      note: "Filmclub",
      expires_in_days: null,
      send_mail: true,
    });
    expect(
      invitationInput({ ...EMPTY_INVITATION_FORM, expiry: "day1" }, false),
    ).toEqual({ expires_in_days: 1 });
    expect(
      invitationInput({ ...EMPTY_INVITATION_FORM, expiry: "day30" }, false),
    ).toEqual({ expires_in_days: 30 });
  });

  it("offers the mail only with an address and mail on this instance", () => {
    const withAddress = {
      ...EMPTY_INVITATION_FORM,
      email: "ada@example.test",
      sendMail: true,
    };
    expect(canSendMail(withAddress, true)).toBe(true);
    expect(canSendMail(withAddress, false)).toBe(false);
    expect(canSendMail({ ...withAddress, email: "  " }, true)).toBe(false);
    // A ticked box that is no longer offered sends nothing.
    expect(invitationInput(withAddress, false)).not.toHaveProperty("send_mail");
  });

  it("checks the address and the note before asking", () => {
    expect(checkInvitationForm(EMPTY_INVITATION_FORM)).toEqual({});
    expect(
      checkInvitationForm({
        ...EMPTY_INVITATION_FORM,
        email: "nicht-gültig",
        note: "x".repeat(201),
      }),
    ).toEqual({ email: "invalid", note: "too_long" });
  });

  it("explains the API's field errors at their fields", () => {
    expect(
      invitationFieldMessages({
        email: "taken",
        note: "invalid",
        send_mail: "unavailable",
        expires_in_days: "invalid",
      }),
    ).toEqual({
      email: "Diese E-Mail-Adresse ist schon registriert.",
      note: "Nur eine Zeile Text, keine Steuerzeichen.",
      send_mail:
        "Per E-Mail geht nur mit einer Adresse und wenn diese Instanz E-Mails verschickt.",
      expires_in_days: "Bitte eine der Laufzeiten wählen.",
    });
    expect(invitationFieldMessages({ note: "too_long" })).toEqual({
      note: "Höchstens 200 Zeichen.",
    });
    setLocale("en", { persist: false });
    expect(invitationFieldMessages({ send_mail: "unavailable" })).toEqual({
      send_mail:
        "Sending by email needs an address and email set up on this instance.",
    });
  });
});

describe("invitation row menu", () => {
  it("copies or revokes an open invitation", () => {
    const on = { copy: vi.fn(), revoke: vi.fn() };
    const entries = invitationMenuEntries({ status: "open" }, on);
    expect(labels(entries)).toEqual(["Link kopieren", "Widerrufen"]);
    for (const entry of entries) if (entry.kind === "item") entry.onSelect();
    expect(on.copy).toHaveBeenCalledOnce();
    expect(on.revoke).toHaveBeenCalledOnce();
    setLocale("en", { persist: false });
    expect(labels(invitationMenuEntries({ status: "open" }, on))).toEqual([
      "Copy link",
      "Revoke",
    ]);
  });

  it("has nothing for a used, expired or revoked one", () => {
    const on = { copy: vi.fn(), revoke: vi.fn() };
    for (const status of ["used", "expired", "revoked"] as const)
      expect(invitationMenuEntries({ status }, on)).toEqual([]);
  });

  it("says whether the copy worked", async () => {
    expect(
      await copyInvitationLink("https://g.test/register?invitation=k"),
    ).toBe(true);
    expect(fixture.copyText).toHaveBeenCalledWith(
      "https://g.test/register?invitation=k",
    );
    expect(fixture.notify).toHaveBeenCalledWith("Link kopiert.");
    fixture.copyText.mockImplementation(async () => false);
    expect(await copyInvitationLink("x")).toBe(false);
    expect(fixture.notifyError).toHaveBeenCalledOnce();
  });

  it("revokes only after asking", async () => {
    const refresh = vi.fn();
    fixture.confirm.mockImplementation(async () => false);
    expect(await revokeInvitation({ id: "i1" }, refresh)).toBe(false);
    expect(fixture.api.revokeInvitation).not.toHaveBeenCalled();

    fixture.confirm.mockImplementation(async () => true);
    expect(await revokeInvitation({ id: "i1" }, refresh)).toBe(true);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({
        title: "Einladung widerrufen?",
        confirmLabel: "Widerrufen",
      }),
    );
    expect(fixture.api.revokeInvitation).toHaveBeenCalledWith("i1");
    expect(fixture.notify).toHaveBeenCalledWith(
      "Die Einladung ist widerrufen.",
    );
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("says when the invitation was used meanwhile", async () => {
    fixture.api.revokeInvitation.mockRejectedValueOnce(
      new ApiError("validation_failed", 422, "used", {
        invitation: "used",
      } as never),
    );
    expect(await revokeInvitation({ id: "i1" }, vi.fn())).toBe(false);
    const [error, failed, gone] = fixture.notifyError.mock.calls[0] ?? [];
    expect((error as Error).message).toBe("Sie wurde schon benutzt.");
    expect(failed).toBe("Einladung nicht widerrufen.");
    expect(gone).toBe("Die Einladung gibt es nicht mehr.");
  });
});
