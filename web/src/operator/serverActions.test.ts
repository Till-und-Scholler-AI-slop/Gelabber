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
    delistServer: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    relistServer: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    blockServer: vi.fn<(id: string, reason: string) => Promise<unknown>>(
      async () => ({}),
    ),
    unblockServer: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    resolveReport: vi.fn<(id: string, input: unknown) => Promise<unknown>>(
      async () => ({}),
    ),
  },
}));
vi.mock("../components/confirm.ts", () => ({ confirmAction: fixture.confirm }));
vi.mock("../components/toasts.ts", () => ({
  notify: fixture.notify,
  notifyError: fixture.notifyError,
}));
vi.mock("./api.ts", () => fixture.api);

import {
  blockRequest,
  delistServer,
  dismissRequest,
  relistServer,
  reportMenuEntries,
  serverMenuEntries,
  unblockServer,
  type ServerTarget,
} from "./serverActions.tsx";

const server: ServerTarget = {
  id: "s1",
  name: "Filmclub",
  directory: "off",
  blocked: false,
};
const handlers = () => ({
  message: vi.fn(),
  delist: vi.fn(),
  relist: vi.fn(),
  block: vi.fn(),
  unblock: vi.fn(),
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

describe("server row menu", () => {
  it("offers the owner, the directory and the block", () => {
    const on = handlers();
    const entries = serverMenuEntries(server, on);
    expect(labels(entries)).toEqual([
      "Inhaber anschreiben",
      "separator",
      "Aus dem Verzeichnis nehmen",
      "Server sperren",
    ]);
    pick(entries, "Inhaber anschreiben").onSelect();
    pick(entries, "Aus dem Verzeichnis nehmen").onSelect();
    pick(entries, "Server sperren").onSelect();
    expect(on.message).toHaveBeenCalledOnce();
    expect(on.delist).toHaveBeenCalledOnce();
    expect(on.block).toHaveBeenCalledOnce();
    expect(pick(entries, "Server sperren").danger).toBe(true);
  });

  it("allows a delisted server again and lifts a block", () => {
    const on = handlers();
    const entries = serverMenuEntries(
      { directory: "delisted", blocked: true },
      { ...on, message: undefined },
    );
    expect(labels(entries)).toEqual(["Wieder zulassen", "Sperre aufheben"]);
    pick(entries, "Wieder zulassen").onSelect();
    pick(entries, "Sperre aufheben").onSelect();
    expect(on.relist).toHaveBeenCalledOnce();
    expect(on.unblock).toHaveBeenCalledOnce();
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    expect(labels(serverMenuEntries(server, handlers()))).toEqual([
      "Message the owner",
      "separator",
      "Remove from the directory",
      "Block server",
    ]);
  });
});

describe("report row menu", () => {
  const report = {
    status: "open" as const,
    server: {
      id: "s1",
      name: "Filmclub",
      icon_url: null,
      blocked: false,
      directory: "off" as const,
    },
  };
  const on = () => ({
    open: vi.fn(),
    dismiss: vi.fn(),
    delist: vi.fn(),
    block: vi.fn(),
  });

  it("closes an open report three ways", () => {
    const actions = on();
    const entries = reportMenuEntries(report, actions);
    expect(labels(entries)).toEqual([
      "Zum Server",
      "separator",
      "Erledigt ohne Folgen",
      "Aus dem Verzeichnis nehmen",
      "Server sperren",
    ]);
    for (const label of labels(entries))
      if (label !== "separator") pick(entries, label).onSelect();
    expect(actions.open).toHaveBeenCalledOnce();
    expect(actions.dismiss).toHaveBeenCalledOnce();
    expect(actions.delist).toHaveBeenCalledOnce();
    expect(actions.block).toHaveBeenCalledOnce();
  });

  it("leaves a closed report only the way to its server", () => {
    expect(
      labels(reportMenuEntries({ ...report, status: "dismissed" }, on())),
    ).toEqual(["Zum Server"]);
  });

  it("offers no block for a blocked server", () => {
    expect(
      labels(
        reportMenuEntries(
          { ...report, server: { ...report.server, blocked: true } },
          on(),
        ),
      ),
    ).not.toContain("Server sperren");
  });
});

describe("server actions", () => {
  it("delists only after the operator confirms", async () => {
    const refresh = vi.fn();
    fixture.confirm.mockImplementation(async () => false);
    expect(await delistServer(server, refresh)).toBe(false);
    expect(fixture.api.delistServer).not.toHaveBeenCalled();
    expect(refresh).not.toHaveBeenCalled();

    fixture.confirm.mockImplementation(async () => true);
    expect(await delistServer(server, refresh)).toBe(true);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({
        title: "Filmclub aus dem Verzeichnis nehmen?",
        confirmLabel: "Aus dem Verzeichnis nehmen",
      }),
    );
    expect(fixture.api.delistServer).toHaveBeenCalledWith("s1");
    expect(fixture.notify).toHaveBeenCalledWith(
      "Filmclub ist aus dem Verzeichnis genommen.",
    );
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("relists and unblocks after a neutral question", async () => {
    const refresh = vi.fn();
    await relistServer(server, refresh);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({ tone: "neutral" }),
    );
    expect(fixture.api.relistServer).toHaveBeenCalledWith("s1");
    await unblockServer(server, refresh);
    expect(fixture.confirm).toHaveBeenLastCalledWith(
      expect.objectContaining({ title: "Sperre von Filmclub aufheben?" }),
    );
    expect(fixture.api.unblockServer).toHaveBeenCalledWith("s1");
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("names what did not happen when the route fails", async () => {
    const refresh = vi.fn();
    const error = new Error("down");
    fixture.api.delistServer.mockRejectedValueOnce(error);
    expect(await delistServer(server, refresh)).toBe(false);
    expect(fixture.notifyError).toHaveBeenCalledWith(
      error,
      "Nicht aus dem Verzeichnis genommen.",
      "Den Server gibt es nicht mehr.",
    );
    expect(fixture.notify).not.toHaveBeenCalled();
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("blocks with the reason from the dialog", async () => {
    const refresh = vi.fn();
    const request = blockRequest(server, refresh);
    expect(request).toMatchObject({
      title: "Filmclub sperren?",
      label: "Grund",
      field: "reason",
      required: true,
      danger: true,
    });
    await request.submit("Spam-Links");
    expect(fixture.api.blockServer).toHaveBeenCalledWith("s1", "Spam-Links");
    expect(fixture.notify).toHaveBeenCalledWith("Filmclub ist gesperrt.");
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("keeps the block dialog's error for the dialog", async () => {
    const refresh = vi.fn();
    fixture.api.blockServer.mockRejectedValueOnce(new Error("422"));
    await expect(blockRequest(server, refresh).submit("x")).rejects.toThrow();
    expect(fixture.notify).not.toHaveBeenCalled();
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("resolves a report without action, with or without a note", async () => {
    const refresh = vi.fn();
    const report = {
      id: "r1",
      server: {
        id: "s1",
        name: "Filmclub",
        icon_url: null,
        blocked: false,
        directory: "off" as const,
      },
    };
    const request = dismissRequest(report, refresh);
    expect(request).toMatchObject({ field: "note", required: false });
    await request.submit("");
    expect(fixture.api.resolveReport).toHaveBeenLastCalledWith("r1", {
      status: "dismissed",
    });
    await request.submit("Kein Verstoß");
    expect(fixture.api.resolveReport).toHaveBeenLastCalledWith("r1", {
      status: "dismissed",
      note: "Kein Verstoß",
    });
    expect(fixture.notify).toHaveBeenCalledWith("Die Meldung ist erledigt.");
    expect(refresh).toHaveBeenCalledTimes(2);
  });
});
