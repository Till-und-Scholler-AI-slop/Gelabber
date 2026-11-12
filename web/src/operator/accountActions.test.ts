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
  disableRequest,
  enableAccount,
  type AccountTarget,
} from "./accountActions.tsx";

const account: AccountTarget = {
  id: "u1",
  name: "Tom",
  status: "active",
  operator: false,
};
const handlers = () => ({
  message: vi.fn(),
  disable: vi.fn(),
  enable: vi.fn(),
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
