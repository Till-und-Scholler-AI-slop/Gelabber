import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api/client.ts";
import { decodeKey, syncPush } from "./push.ts";

vi.mock("../api/client.ts", () => ({ api: vi.fn() }));
vi.mock("./register.ts", () => ({ activeServiceWorker: vi.fn() }));
const request = vi.mocked(api);

const KEY = "BAEC";
const OTHER = "BAED";

function pushManager(existing?: Uint8Array) {
  const make = (key: Uint8Array) => ({
    options: { applicationServerKey: key.buffer },
    unsubscribe: vi.fn(async () => true),
    toJSON: () => ({
      endpoint: "https://fcm.googleapis.com/fcm/send/x",
      keys: { p256dh: "p", auth: "a" },
    }),
  });
  const current = existing ? make(existing) : null;
  return {
    current,
    getSubscription: vi.fn(async () => current),
    subscribe: vi.fn(
      async ({ applicationServerKey }: { applicationServerKey: Uint8Array }) =>
        make(applicationServerKey),
    ),
  };
}

describe("Web Push subscription", () => {
  beforeEach(() => {
    vi.stubGlobal("Notification", { permission: "granted" });
    request.mockReset();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("decodes the server key from base64url", () => {
    expect([...decodeKey("-_8")]).toEqual([0xfb, 0xff]);
  });

  it("subscribes with the server key and hands the subscription over", async () => {
    request.mockResolvedValueOnce({ key: KEY }).mockResolvedValueOnce(null);
    const manager = pushManager();
    expect(await syncPush({ pushManager: manager } as never)).toBe("on");
    expect(manager.subscribe).toHaveBeenCalledWith({
      userVisibleOnly: true,
      applicationServerKey: decodeKey(KEY),
    });
    expect(request).toHaveBeenLastCalledWith("/push/subscriptions", {
      method: "POST",
      body: {
        endpoint: "https://fcm.googleapis.com/fcm/send/x",
        keys: { p256dh: "p", auth: "a" },
      },
    });
  });

  it("keeps a subscription of the same key and replaces one of another", async () => {
    request.mockResolvedValue({ key: KEY });
    const same = pushManager(decodeKey(KEY));
    await syncPush({ pushManager: same } as never);
    expect(same.subscribe).not.toHaveBeenCalled();
    const stale = pushManager(decodeKey(OTHER));
    await syncPush({ pushManager: stale } as never);
    expect(stale.current?.unsubscribe).toHaveBeenCalled();
    expect(stale.subscribe).toHaveBeenCalled();
  });

  it("does nothing without permission, push support or a server key", async () => {
    request.mockResolvedValue({ key: null });
    expect(await syncPush({ pushManager: pushManager() } as never)).toBe(
      "unavailable",
    );
    expect(await syncPush({} as never)).toBe("unavailable");
    vi.stubGlobal("Notification", { permission: "default" });
    expect(await syncPush({ pushManager: pushManager() } as never)).toBe("off");
    expect(request).toHaveBeenCalledTimes(1);
  });
});
