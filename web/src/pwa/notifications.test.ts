import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  type Mock,
  vi,
} from "vitest";

import {
  closeMessageNotifications,
  followNotificationTaps,
  NOTIFICATION_OPEN,
  showMessageNotification,
  silenceWhileViewing,
} from "./notifications.ts";
import { useInstallation } from "./install.ts";
import { activeServiceWorker } from "./register.ts";

vi.mock("./register.ts", () => ({ activeServiceWorker: vi.fn() }));
const registration = vi.mocked(activeServiceWorker);

// Windows that say they show a conversation; each test's are closed after it.
const windows: (() => void)[] = [];

beforeEach(() => {
  registration.mockReset().mockResolvedValue(undefined);
  useInstallation.setState({ mobile: false });
});
afterEach(async () => {
  vi.useRealTimers();
  for (const close of windows.splice(0)) close();
  await closeMessageNotifications({ user: "user-1" });
  vi.unstubAllGlobals();
});

const message = {
  title: "Ada · #allgemein",
  body: "Hallo",
  channelId: "chan",
  messageId: "msg-1",
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

function worker(
  shown: { tag: string; data?: unknown; close: () => void }[] = [],
) {
  const active = {
    showNotification: vi.fn().mockResolvedValue(undefined),
    getNotifications: vi.fn(async (filter?: { tag: string }) =>
      shown.filter((one) => !filter || one.tag === filter.tag),
    ),
  };
  registration.mockResolvedValue(
    active as unknown as ServiceWorkerRegistration,
  );
  return active;
}

describe("message notifications", () => {
  // On a phone the notification is the only thing that tells of a message:
  // the app has no message sound, and the phone is in a pocket. `silent` was
  // carried over from the desktop path, where the banner pops up anyway.
  it("goes through the service worker on a phone, with sound and for every message", async () => {
    useInstallation.setState({ mobile: true });
    const created = pageApi("granted", true);
    const active = worker();
    const onClick = vi.fn();
    expect(await showMessageNotification(message, onClick)).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledExactlyOnceWith(
      "Ada · #allgemein",
      {
        body: "Hallo",
        silent: false,
        tag: "gelabber:user-1:chan",
        // The next message of the conversation replaces this one. Without
        // `renotify` it would do so without a sound.
        renotify: true,
        icon: "/icons/icon-192.png",
        data: { path: "/s/srv/c/chan", user: "user-1", message: "msg-1" },
      },
    );
    expect(created).toEqual([]);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("stays silent through the worker of a desktop browser, as before", async () => {
    const created = pageApi();
    const active = worker();
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledExactlyOnceWith(
      "Ada · #allgemein",
      {
        body: "Hallo",
        silent: true,
        tag: "gelabber:user-1:chan",
        renotify: false,
        icon: "/icons/icon-192.png",
        data: { path: "/s/srv/c/chan", user: "user-1", message: "msg-1" },
      },
    );
    expect(created).toEqual([]);
  });

  // Every window of the account hears the message. Shown twice it would
  // sound twice on a phone.
  it("announces a message once, whichever window is first", async () => {
    useInstallation.setState({ mobile: true });
    pageApi("granted", true);
    const onScreen: { tag: string; data?: unknown; close: () => void }[] = [];
    const active = worker(onScreen);
    active.showNotification.mockImplementation(
      async (_title: string, options: { tag: string; data: unknown }) => {
        // As a browser does it: the list has it once the promise resolves.
        await new Promise((resolve) => setTimeout(resolve, 5));
        const same = onScreen.findIndex((one) => one.tag === options.tag);
        if (same >= 0) onScreen.splice(same, 1);
        onScreen.push({ tag: options.tag, data: options.data, close: vi.fn() });
      },
    );
    // Two windows at the same moment.
    expect(
      await Promise.all([
        showMessageNotification(message, vi.fn()),
        showMessageNotification(message, vi.fn()),
      ]),
    ).toEqual(["worker", "none"]);
    // One that hears it later, as after a reconnect.
    expect(await showMessageNotification(message, vi.fn())).toBe("none");
    expect(active.showNotification).toHaveBeenCalledOnce();

    // The next message of the conversation, and another conversation.
    const next = { ...message, messageId: "msg-2", body: "Noch da?" };
    expect(await showMessageNotification(next, vi.fn())).toBe("worker");
    expect(
      await showMessageNotification({ ...message, channelId: "dm" }, vi.fn()),
    ).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledTimes(3);
    expect(onScreen.map((one) => one.data)).toEqual([
      { path: "/s/srv/c/chan", user: "user-1", message: "msg-2" },
      { path: "/s/srv/c/chan", user: "user-1", message: "msg-1" },
    ]);
  });

  it("shows a message next to notifications that name none, and where the worker cannot list them", async () => {
    pageApi();
    const older = { tag: "gelabber:user-1:chan", close: vi.fn() };
    const active = worker([older, { ...older, data: null }]);
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    active.getNotifications.mockRejectedValue(new TypeError("not here"));
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledTimes(2);
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
    void closeMessageNotifications({ user: "user-1" });
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

  // What the worker shows belongs to every window of the app. Closing more
  // than the account that left took the other conversations' notifications
  // away each time a window started.
  it("closes the notifications of the account that leaves and no others", async () => {
    const mine = { tag: "gelabber:user-1:chan", close: vi.fn() };
    const alsoMine = { tag: "gelabber:user-1:dm", close: vi.fn() };
    const theirs = { tag: "gelabber:user-12:chan", close: vi.fn() };
    const foreign = { tag: "something-else", close: vi.fn() };
    worker([mine, alsoMine, theirs, foreign]);
    closeMessageNotifications({ user: "user-1" });
    await vi.waitFor(() => expect(alsoMine.close).toHaveBeenCalledOnce());
    expect(mine.close).toHaveBeenCalledOnce();
    expect(theirs.close).not.toHaveBeenCalled();
    expect(foreign.close).not.toHaveBeenCalled();
  });

  it("withdraws one conversation's notification and leaves the others", async () => {
    const read = { tag: "gelabber:user-1:chan", close: vi.fn() };
    const longer = { tag: "gelabber:user-1:chan-2", close: vi.fn() };
    const theirs = { tag: "gelabber:user-2:chan", close: vi.fn() };
    const active = worker([read, longer, theirs]);
    closeMessageNotifications({ user: "user-1", channelId: "chan" });
    await vi.waitFor(() => expect(read.close).toHaveBeenCalledOnce());
    expect(active.getNotifications).toHaveBeenCalledOnce();
    expect(longer.close).not.toHaveBeenCalled();
    expect(theirs.close).not.toHaveBeenCalled();
  });

  it("withdraws page-level notifications the same way", async () => {
    const created = pageApi();
    await showMessageNotification(message, vi.fn());
    await showMessageNotification({ ...message, channelId: "dm" }, vi.fn());
    closeMessageNotifications({ user: "user-2" });
    closeMessageNotifications({ user: "user-1", channelId: "d" });
    expect(created.map((shown) => shown.close.mock.calls.length)).toEqual([
      0, 0,
    ]);
    closeMessageNotifications({ user: "user-1", channelId: "dm" });
    expect(created.map((shown) => shown.close.mock.calls.length)).toEqual([
      0, 1,
    ]);
    // Closed once; the page no longer holds it.
    closeMessageNotifications({ user: "user-1" });
    expect(created.map((shown) => shown.close.mock.calls.length)).toEqual([
      1, 1,
    ]);
  });

  it("survives a browser whose worker cannot list notifications", async () => {
    registration.mockResolvedValue({} as ServiceWorkerRegistration);
    closeMessageNotifications({ user: "user-1" });
    await vi.waitFor(() => expect(registration).toHaveBeenCalledOnce());
  });
});

describe("a conversation on screen", () => {
  function page(hidden: boolean) {
    const target = new EventTarget();
    const state = { hidden };
    return {
      get hidden() {
        return state.hidden;
      },
      addEventListener: target.addEventListener.bind(target),
      removeEventListener: target.removeEventListener.bind(target),
      show(visible: boolean) {
        state.hidden = !visible;
        target.dispatchEvent(new Event("visibilitychange"));
      },
    };
  }
  /** A window of the app with the conversation open. */
  function viewer(user: string, channelId: string, visible: boolean) {
    const tab = page(!visible);
    const stop = silenceWhileViewing(user, channelId, tab);
    windows.push(stop);
    return { show: tab.show, stop };
  }
  const shown = () => ({
    viewed: { tag: "gelabber:user-1:chan", close: vi.fn() },
    other: { tag: "gelabber:user-1:dm", close: vi.fn() },
  });

  it("loses its notification at once in a visible page", async () => {
    const { viewed, other } = shown();
    worker([viewed, other]);
    viewer("user-1", "chan", true);
    await vi.waitFor(() => expect(viewed.close).toHaveBeenCalledOnce());
    expect(other.close).not.toHaveBeenCalled();
  });

  // The phone case: the notification arrives while the app is in the
  // background, and the user comes back through the app switcher.
  it("loses it when a hidden page comes back, and only then", async () => {
    const { viewed, other } = shown();
    worker([viewed, other]);
    const background = viewer("user-1", "chan", false);
    background.show(false);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(registration).not.toHaveBeenCalled();
    background.show(true);
    await vi.waitFor(() => expect(viewed.close).toHaveBeenCalledOnce());
    expect(other.close).not.toHaveBeenCalled();

    // Another conversation is opened: this one is no longer watched.
    background.stop();
    background.show(true);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(registration).toHaveBeenCalledOnce();
  });

  // Every window hears the same message and decides from its own visibility.
  // The one in the background (a browser tab left behind after installing
  // the app, a second tab) must not notify about what is being read in the
  // one in front.
  it("raises no notification from another window while it is visible", async () => {
    pageApi();
    const active = worker();
    const front = viewer("user-1", "chan", true);
    expect(await showMessageNotification(message, vi.fn())).toBe("viewed");
    expect(active.showNotification).not.toHaveBeenCalled();

    // Other conversations and other accounts are not on screen.
    expect(
      await showMessageNotification({ ...message, channelId: "dm" }, vi.fn()),
    ).toBe("worker");
    expect(
      await showMessageNotification({ ...message, user: "user-2" }, vi.fn()),
    ).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledTimes(2);

    // In the background itself, or on another conversation: no longer read.
    front.show(false);
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    front.show(true);
    expect(await showMessageNotification(message, vi.fn())).toBe("viewed");
    front.stop();
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledTimes(4);
  });

  it("keeps a page-level notification back the same way", async () => {
    const created = pageApi();
    viewer("user-1", "chan", true);
    expect(await showMessageNotification(message, vi.fn())).toBe("viewed");
    expect(created).toEqual([]);
  });

  it("counts every visible window that shows the conversation", async () => {
    pageApi();
    const active = worker();
    const one = viewer("user-1", "chan", true);
    const two = viewer("user-1", "chan", true);
    one.show(false);
    expect(await showMessageNotification(message, vi.fn())).toBe("viewed");
    two.stop();
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledOnce();
  });

  // A window in the background is showing a notification at the moment the
  // conversation comes on screen in another one. Closing must wait for it,
  // or the notification appears after the look and stays.
  it("takes away what another window is just showing as it comes on screen", async () => {
    pageApi();
    const onScreen: { tag: string; close: Mock<() => void> }[] = [];
    const active = worker(onScreen);
    let landed = () => {};
    active.showNotification.mockImplementation(
      (_title: string, options: { tag: string }) =>
        new Promise<void>((resolve) => {
          landed = () => {
            onScreen.push({ tag: options.tag, close: vi.fn<() => void>() });
            resolve();
          };
        }),
    );
    const showing = showMessageNotification(message, vi.fn());
    try {
      await vi.waitFor(() =>
        expect(active.showNotification).toHaveBeenCalledOnce(),
      );
      // The one look so far is the showing window's own, for this message.
      expect(active.getNotifications).toHaveBeenCalledOnce();
      viewer("user-1", "chan", true);
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(active.getNotifications).toHaveBeenCalledOnce();
    } finally {
      // Whatever fails above, the window lets go of the account's lock.
      landed();
    }
    expect(await showing).toBe("worker");
    await vi.waitFor(() => expect(onScreen[0].close).toHaveBeenCalledOnce());
  });

  // Showing and closing wait for each other across windows. A browser that
  // never answers one window's request must not stop the others for good.
  it("stops waiting for a window whose notification never lands", async () => {
    const later = setTimeout;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    pageApi();
    const { viewed } = shown();
    const active = worker([viewed]);
    active.showNotification.mockReturnValueOnce(new Promise(() => {}));
    void showMessageNotification({ ...message, channelId: "dm" }, vi.fn());
    // On the real clock: vi.waitFor would move the stopped one along.
    while (active.showNotification.mock.calls.length === 0)
      await new Promise((resolve) => later(resolve, 1));
    const closing = closeMessageNotifications({ user: "user-1" });
    await vi.advanceTimersByTimeAsync(4_000);
    expect(viewed.close).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1_500);
    await closing;
    expect(viewed.close).toHaveBeenCalledOnce();
  });

  it("works as before where the browser has no lock manager", async () => {
    vi.stubGlobal("navigator", {});
    pageApi();
    const { viewed } = shown();
    const active = worker([viewed]);
    viewer("user-1", "chan", true);
    await vi.waitFor(() => expect(viewed.close).toHaveBeenCalledOnce());
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledOnce();
  });

  it("works as before where the lock manager refuses", async () => {
    const refused = () =>
      Promise.reject(new DOMException("denied", "SecurityError"));
    vi.stubGlobal("navigator", { locks: { request: refused, query: refused } });
    pageApi();
    const { viewed } = shown();
    const active = worker([viewed]);
    viewer("user-1", "chan", true);
    await vi.waitFor(() => expect(viewed.close).toHaveBeenCalledOnce());
    expect(await showMessageNotification(message, vi.fn())).toBe("worker");
    expect(active.showNotification).toHaveBeenCalledOnce();
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
