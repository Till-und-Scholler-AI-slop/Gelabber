import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  confirm: vi.fn<
    (request: { title: string; tone?: string }) => Promise<boolean>
  >(async () => true),
  notify: vi.fn(),
  notifyError: vi.fn(),
  api: {
    disableAccount: vi.fn<(id: string, reason?: string) => Promise<unknown>>(
      async () => ({}),
    ),
    enableAccount: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    resendConfirmation: vi.fn<(id: string) => Promise<unknown>>(
      async () => ({}),
    ),
    confirmAccount: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
  },
}));
vi.mock("../components/confirm.ts", () => ({ confirmAction: fixture.confirm }));
vi.mock("../components/toasts.ts", () => ({
  notify: fixture.notify,
  notifyError: fixture.notifyError,
}));
vi.mock("./api.ts", () => fixture.api);

import {
  accountMenuEntries,
  confirmAddress,
  disableRequest,
  enableAccount,
  resendConfirmation,
  type AccountTarget,
} from "./accountActions.tsx";

const account: AccountTarget = {
  id: "u1",
  name: "Tom",
  status: "active",
  operator: false,
  email_verified: true,
};
const handlers = () => ({
  message: vi.fn(),
  disable: vi.fn(),
  enable: vi.fn(),
  resend: vi.fn(),
  confirm: vi.fn(),
});
const labels = (entries: MenuEntry[]) =>
  entries.map((entry) => (entry.kind === "item" ? entry.label : entry.kind));
const pick = (entries: MenuEntry[], label: string) => {
  const entry = entries.find(
    (candidate) => candidate.kind === "item" && candidate.label === label,
  );
  if (entry?.kind !== "item") throw new Error(`no entry ${label}`);
  return entry;
};

beforeEach(() => {
  vi.clearAllMocks();
  fixture.confirm.mockImplementation(async () => true);
});
afterEach(() => setLocale("de", { persist: false }));

describe("account row menu", () => {
  it("offers a message and the disable", () => {
    const on = handlers();
    const entries = accountMenuEntries(account, on);
    expect(labels(entries)).toEqual([
      "Direktnachricht",
      "separator",
      "Konto sperren",
    ]);
    pick(entries, "Direktnachricht").onSelect();
    pick(entries, "Konto sperren").onSelect();
    expect(on.message).toHaveBeenCalledOnce();
    expect(on.disable).toHaveBeenCalledOnce();
    expect(pick(entries, "Konto sperren").danger).toBe(true);
  });

  it("enables a disabled account", () => {
    const on = handlers();
    const entries = accountMenuEntries({ ...account, status: "disabled" }, on);
    expect(labels(entries)).toEqual([
      "Direktnachricht",
      "separator",
      "Konto entsperren",
    ]);
    pick(entries, "Konto entsperren").onSelect();
    expect(on.enable).toHaveBeenCalledOnce();
  });

  it("never disables an operator, and has nothing for your own row", () => {
    expect(
      labels(accountMenuEntries({ ...account, operator: true }, handlers())),
    ).toEqual(["Direktnachricht"]);
    expect(
      accountMenuEntries(
        { ...account, operator: true },
        { ...handlers(), message: undefined },
      ),
    ).toEqual([]);
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    expect(labels(accountMenuEntries(account, handlers()))).toEqual([
      "Direct message",
      "separator",
      "Disable account",
    ]);
  });

  it("helps an address that was never confirmed (GEL-40)", () => {
    const on = handlers();
    const unconfirmed = {
      ...account,
      status: "unconfirmed" as const,
      email_verified: false,
    };
    const entries = accountMenuEntries(unconfirmed, on);
    expect(labels(entries)).toEqual([
      "Direktnachricht",
      "separator",
      "Bestätigungsmail senden",
      "Adresse als bestätigt markieren",
      "separator",
      "Konto sperren",
    ]);
    pick(entries, "Bestätigungsmail senden").onSelect();
    pick(entries, "Adresse als bestätigt markieren").onSelect();
    expect(on.resend).toHaveBeenCalledOnce();
    expect(on.confirm).toHaveBeenCalledOnce();
    // A pending registration gets the link but is never marked confirmed:
    // its password may be a stranger's.
    expect(
      labels(accountMenuEntries({ ...unconfirmed, status: "pending" }, on)),
    ).toEqual([
      "Direktnachricht",
      "separator",
      "Bestätigungsmail senden",
      "separator",
      "Konto sperren",
    ]);
    expect(
      labels(
        accountMenuEntries(
          { ...unconfirmed, status: "pending" },
          { ...on, resend: undefined },
        ),
      ),
    ).toEqual(["Direktnachricht", "separator", "Konto sperren"]);
    // Without account mail there is nothing to send.
    expect(
      labels(accountMenuEntries(unconfirmed, { ...on, resend: undefined })),
    ).toEqual([
      "Direktnachricht",
      "separator",
      "Adresse als bestätigt markieren",
      "separator",
      "Konto sperren",
    ]);
    // A disabled account gets neither: its links would not sign it in.
    expect(
      labels(accountMenuEntries({ ...unconfirmed, status: "disabled" }, on)),
    ).toEqual(["Direktnachricht", "separator", "Konto entsperren"]);
  });
});

describe("account actions", () => {
  it("disables with an optional reason from the dialog", async () => {
    const refresh = vi.fn();
    const request = disableRequest(account, refresh);
    expect(request).toMatchObject({
      title: "Tom sperren?",
      label: "Grund (optional)",
      field: "reason",
      required: false,
      danger: true,
      confirmLabel: "Konto sperren",
    });
    await request.submit("");
    expect(fixture.api.disableAccount).toHaveBeenLastCalledWith(
      "u1",
      undefined,
    );
    await request.submit("Spam");
    expect(fixture.api.disableAccount).toHaveBeenLastCalledWith("u1", "Spam");
    expect(fixture.notify).toHaveBeenCalledWith("Tom ist gesperrt.");
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("keeps the disable dialog's error for the dialog", async () => {
    const refresh = vi.fn();
    fixture.api.disableAccount.mockRejectedValueOnce(new Error("403"));
    await expect(disableRequest(account, refresh).submit("")).rejects.toThrow();
    expect(fixture.notify).not.toHaveBeenCalled();
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("enables only after a neutral question", async () => {
    const refresh = vi.fn();
    fixture.confirm.mockImplementation(async () => false);
    expect(await enableAccount(account, refresh)).toBe(false);
    expect(fixture.api.enableAccount).not.toHaveBeenCalled();

    fixture.confirm.mockImplementation(async () => true);
    expect(await enableAccount(account, refresh)).toBe(true);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({ title: "Tom entsperren?", tone: "neutral" }),
    );
    expect(fixture.api.enableAccount).toHaveBeenCalledWith("u1");
    expect(fixture.notify).toHaveBeenCalledWith("Tom ist entsperrt.");
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("resends the confirmation mail after a neutral question", async () => {
    const refresh = vi.fn();
    expect(await resendConfirmation(account, refresh)).toBe(true);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({
        title: "Bestätigungsmail an Tom senden?",
        tone: "neutral",
      }),
    );
    expect(fixture.api.resendConfirmation).toHaveBeenCalledWith("u1");
    expect(fixture.notify).toHaveBeenCalledWith(
      "Bestätigungsmail an Tom ist unterwegs.",
    );
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("marks an address confirmed only after saying what that vouches for", async () => {
    const refresh = vi.fn();
    fixture.confirm.mockImplementation(async () => false);
    expect(await confirmAddress(account, refresh)).toBe(false);
    expect(fixture.api.confirmAccount).not.toHaveBeenCalled();
    const question = fixture.confirm.mock.lastCall?.[0] as {
      title: string;
      message: string;
    };
    expect(question.title).toBe("Adresse von Tom als bestätigt markieren?");
    expect(question.message).toContain("Passwort");

    fixture.confirm.mockImplementation(async () => true);
    expect(await confirmAddress(account, refresh)).toBe(true);
    expect(fixture.api.confirmAccount).toHaveBeenCalledWith("u1");
    expect(fixture.notify).toHaveBeenCalledWith(
      "Die Adresse von Tom gilt als bestätigt.",
    );
  });

  it("names what did not happen when enabling fails", async () => {
    const error = new Error("down");
    fixture.api.enableAccount.mockRejectedValueOnce(error);
    expect(await enableAccount(account, vi.fn())).toBe(false);
    expect(fixture.notifyError).toHaveBeenCalledWith(
      error,
      "Konto nicht entsperrt.",
      "Das Konto gibt es nicht mehr.",
    );
  });
});
