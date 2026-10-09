import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  closeMessageNotifications,
  followNotificationTaps,
  NOTIFICATION_OPEN,
  showMessageNotification,
} from "./notifications.ts";
import { activeServiceWorker } from "./register.ts";

vi.mock("./register.ts", () => ({ activeServiceWorker: vi.fn() }));
const registration = vi.mocked(activeServiceWorker);

beforeEach(() => {
  registration.mockReset().mockResolvedValue(undefined);
});
afterEach(() => {
  closeMessageNotifications();
  vi.unstubAllGlobals();
});

const message = {
  title: "Ada · #allgemein",
  body: "Hallo",
  tag: "gelabber:user-1:chan",
  path: "/s/srv/c/chan",
  user: "user-1",
};

/** Page-level API as browsers expose it; phones throw from the constructor. */
function pageApi(permission = "granted", phone = false) {
  const created: {
    title: string;
    options: Record<string, unknown>;
    onclick: (() => void) | null;
    close: ReturnType<typeof vi.fn>;
  }[] = [];
  class Api {
    static permission = permission;
    onclick: (() => void) | null = null;
    close = vi.fn();
    constructor(
      public title: string,
      public options: Record<string, unknown>,
    ) {
      if (phone) throw new TypeError("Illegal constructor");
      created.push(this);
    }
  }
  vi.stubGlobal("Notification", Api);
  return created;
}

function worker(shown: { tag: string; close: () => void }[] = []) {
  const active = {
    showNotification: vi.fn().mockResolvedValue(undefined),
    getNotifications: vi.fn().mockResolvedValue(shown),
  };
  registration.mockResolvedValue(
    active as unknown as ServiceWorkerRegistration,
  );
  return active;
}

describe("message notifications", () => {
  it("goes through the service worker where one is active, as phones require", async () => {
    const created = pageApi("granted", true);
    const active = worker();
    const onClick = vi.fn();
    expect(await showMessageNotification(message, onClick)).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledExactlyOnceWith(
      "Ada · #allgemein",
      {
        body: "Hallo",
        silent: true,
        tag: "gelabber:user-1:chan",
        renotify: false,
        icon: "/icons/icon-192.png",
        data: { path: "/s/srv/c/chan", user: "user-1" },
      },
    );
    expect(created).toEqual([]);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("keeps the page-level notification where no worker runs", async () => {
    const created = pageApi();
    registration.mockResolvedValue(undefined);
    const onClick = vi.fn();
    expect(await showMessageNotification(message, onClick)).toBe("page");
    expect(created).toHaveLength(1);
    expect(created[0]).toMatchObject({
      title: "Ada · #allgemein",
      options: {
        body: "Hallo",
        silent: true,
        tag: "gelabber:user-1:chan",
        renotify: false,
      },
    });
    created[0].onclick?.();
    expect(created[0].close).toHaveBeenCalledOnce();
    expect(onClick).toHaveBeenCalledOnce();

    // A newer message in the same conversation replaces the older one.
    await showMessageNotification({ ...message, body: "Noch da?" }, onClick);
    expect(created).toHaveLength(2);
    expect(created[0].close).toHaveBeenCalledTimes(2);
    closeMessageNotifications();
    expect(created[1].close).toHaveBeenCalledOnce();
  });

  it("falls back to the page when the worker refuses", async () => {
    const created = pageApi();
    worker().showNotification.mockRejectedValue(new TypeError("no worker"));
    expect(await showMessageNotification(message, vi.fn())).toBe("page");
    expect(created).toHaveLength(1);
  });

  it("stays silent without permission, without the API and on a phone without a worker", async () => {
    const active = worker();
    expect(await showMessageNotification(message, vi.fn())).toBe("none");
    for (const permission of ["default", "denied"]) {
      const created = pageApi(permission);
      expect(await showMessageNotification(message, vi.fn())).toBe("none");
      expect(created).toEqual([]);
    }
    expect(active.showNotification).not.toHaveBeenCalled();
    expect(registration).not.toHaveBeenCalled();

    pageApi("granted", true);
    registration.mockResolvedValue(undefined);
    expect(await showMessageNotification(message, vi.fn())).toBe("none");
  });

  it("closes only Gelabber's worker notifications when the account changes", async () => {
    const mine = { tag: "gelabber:user-1:chan", close: vi.fn() };
    const other = { tag: "something-else", close: vi.fn() };
    worker([mine, other]);
    closeMessageNotifications();
    await vi.waitFor(() => expect(mine.close).toHaveBeenCalledOnce());
    expect(other.close).not.toHaveBeenCalled();
  });
});

describe("notification taps", () => {
  function app(user: string | null = "user-1") {
    const serviceWorker = new EventTarget();
    vi.stubGlobal("navigator", { serviceWorker });
    const open = vi.fn();
    const stop = followNotificationTaps({
      user: () => user ?? undefined,
      open,
    });
    const tap = (data: unknown) =>
      serviceWorker.dispatchEvent(new MessageEvent("message", { data }));
    return { open, stop, tap };
  }
  const tapped = {
    type: NOTIFICATION_OPEN,
    path: "/s/srv/c/chan",
    user: "user-1",
  };

  it("opens the conversation the worker names", () => {
    const { open, stop, tap } = app();
    tap(tapped);
    tap({ ...tapped, path: "/d/dm-1" });
    expect(open.mock.calls).toEqual([["/s/srv/c/chan"], ["/d/dm-1"]]);
    stop();
    tap(tapped);
    expect(open).toHaveBeenCalledTimes(2);
  });

  it("ignores other messages, other addresses and other accounts", () => {
    const { open, tap } = app();
    for (const data of [
      null,
      "text",
      { type: "something-else", path: "/d/dm-1", user: "user-1" },
      { ...tapped, path: "/" },
      { ...tapped, path: "https://evil.example/d/x" },
      { ...tapped, path: "//evil.example/d/x" },
      { ...tapped, user: "user-2" },
      { ...tapped, user: undefined },
    ]) {
      tap(data);
    }
    expect(open).not.toHaveBeenCalled();
    const loggedOut = app(null);
    loggedOut.tap({ ...tapped, user: undefined });
    loggedOut.tap(tapped);
    expect(loggedOut.open).not.toHaveBeenCalled();
  });

  it("does nothing where the browser has no service workers", () => {
    vi.stubGlobal("navigator", {});
    expect(
      followNotificationTaps({ user: () => "user-1", open: vi.fn() }),
    ).toBeTypeOf("function");
  });
});
