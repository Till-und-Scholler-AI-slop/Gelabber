import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api/client.ts";
import { clearToasts, useToasts } from "../components/toasts.ts";
import { cacheOpenedDm, dmKeys } from "../dms/queries.ts";
import { lastDmId, useLastDm } from "../dms/lastDm.ts";
import type { DirectMessage } from "../dms/types.ts";
import { addPending, usePendingMessages } from "../messages/pending.ts";
import { messageKeys } from "../messages/queries.ts";
import { useMessageToasts } from "../messages/toasts.ts";
import type { Message } from "../messages/types.ts";
import { queryClient } from "../queryClient.ts";
import { serverKeys } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { resetThemeForTests, useTheme } from "../theme/theme.ts";
import {
  resetMediaSettingsForTests,
  useMediaSettings,
} from "../voice/settings.ts";
import { getGateway, resetGatewayForTests } from "../ws/client.ts";
import { takeStamp } from "./scope.ts";
import {
  login,
  logout,
  resetSessionForTests,
  updateProfile,
  useSession,
} from "./session.ts";
import type { User } from "./types.ts";

const ada: User = {
  id: "user-a",
  email: "ada@example.com",
  name: "Ada",
  avatar_url: null,
  created_at: "2026-09-13T00:00:00Z",
};

const bob: User = {
  id: "user-b",
  email: "bob@example.com",
  name: "Bob",
  avatar_url: null,
  created_at: "2026-09-13T00:00:00Z",
};

const secretDm: DirectMessage = {
  id: "dm-secret",
  kind: "dm",
  created_at: "2026-09-13T00:00:00Z",
  peer: { id: "peer-a", name: "secret-from-a", avatar_url: null },
};

const secretServer: Server = {
  id: "srv-a",
  name: "secret-server-a",
  owner_id: ada.id,
  created_at: "2026-09-13T00:00:00Z",
  role: "owner",
  permissions: [],
  member_permissions: [],
};

function secretMessage(): Message {
  return {
    id: "msg-a",
    channel_id: "chan-a",
    author: { id: ada.id, name: "Ada", avatar_url: null },
    content: "secret-from-a",
    created_at: "2026-09-13T00:00:00Z",
    edited_at: null,
    attachments: [],
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function cacheDump(): string {
  return JSON.stringify(
    queryClient
      .getQueryCache()
      .getAll()
      .map((query) => ({
        key: query.queryKey,
        data: query.state.data,
      })),
  );
}

describe("account switch drops the previous user's state", () => {
  let who: User = ada;

  beforeEach(() => {
    resetSessionForTests();
    resetMediaSettingsForTests();
    resetThemeForTests();
    useLastDm.setState({ byUser: {} });
    who = ada;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const key = `${init?.method ?? "GET"} ${String(input)}`;
        if (key === "POST /api/auth/login") {
          return Promise.resolve(json(200, { user: who, csrf_token: "csrf" }));
        }
        if (key === "POST /api/auth/logout") {
          return Promise.resolve(json(200, { csrf_token: "after-logout" }));
        }
        if (key === "PATCH /api/me") {
          return Promise.resolve(json(200, { ...ada, name: "Ada Lovelace" }));
        }
        if (key === "GET /api/servers") {
          return Promise.resolve(
            json(401, {
              error: "unauthenticated",
              message: "Sign in required.",
            }),
          );
        }
        return Promise.reject(new Error(`unexpected request ${key}`));
      }),
    );
  });

  afterEach(() => {
    resetSessionForTests();
    resetGatewayForTests();
    clearToasts();
    vi.unstubAllGlobals();
  });

  it("hides A's DMs, servers, and messages from B, including a late response", async () => {
    await login(ada.email, "password123");
    const generation = takeStamp()?.generation;
    expect(generation).toEqual(expect.any(Number));

    queryClient.setQueryData(serverKeys.list(ada.id, generation ?? 0), [
      secretServer,
    ]);
    queryClient.setQueryData(
      messageKeys.channel(ada.id, generation ?? 0, "chan-a"),
      {
        pages: [{ messages: [secretMessage()], has_more: false }],
        pageParams: [undefined],
      },
    );
    queryClient.setQueryData(dmKeys.list(ada.id, generation ?? 0), [secretDm]);

    let release: ((rows: DirectMessage[]) => void) | undefined;
    const flight = queryClient.fetchQuery({
      queryKey: dmKeys.list(ada.id, generation ?? 0),
      retry: false,
      queryFn: () =>
        new Promise<DirectMessage[]>((resolve) => {
          release = resolve;
        }),
    });
    const stamp = takeStamp();

    addPending("chan-a", secretMessage());
    useMessageToasts.getState().push({
      channelId: "chan-a",
      serverId: "srv-a",
      dm: false,
      channelLabel: "#secret",
      author: "Ada",
      preview: "secret-from-a",
    });
    useToasts.getState().push("info", "invite secret-from-a");
    useLastDm.getState().remember(ada.id, "dm-secret");
    useMediaSettings
      .getState()
      .patch({ audioInputId: "mic-a", quality: "high" });
    useTheme.getState().setPreference("dark");
    getGateway().setTopics([{ s: "srv-a", c: "chan-a" }]);

    await logout();

    expect(cacheDump()).not.toContain("secret-from-a");
    expect(cacheDump()).not.toContain("secret-server-a");
    expect(usePendingMessages.getState().byChannel).toEqual({});
    expect(useMessageToasts.getState().toasts).toEqual([]);
    expect(useToasts.getState().toasts).toEqual([]);
    expect(getGateway().cursorsSnapshot.size).toBe(0);
    expect(useMediaSettings.getState().audioInputId).toBe("mic-a");
    expect(useMediaSettings.getState().quality).toBe("high");
    expect(useTheme.getState().preference).toBe("dark");
    expect(lastDmId(useLastDm.getState().byUser, ada.id)).toBe("dm-secret");
    expect(lastDmId(useLastDm.getState().byUser, bob.id)).toBeNull();

    who = bob;
    await login(bob.email, "password123");

    expect(
      queryClient.getQueryData(
        dmKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();
    expect(
      queryClient.getQueryData(
        serverKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();
    expect(cacheDump()).not.toContain("secret-from-a");
    expect(cacheDump()).not.toContain("secret-server-a");

    release?.([
      {
        ...secretDm,
        peer: { ...secretDm.peer, name: "late-secret-from-a" },
      },
    ]);
    await flight.catch(() => undefined);
    cacheOpenedDm(queryClient, stamp, secretDm);
    await Promise.resolve();
    await Promise.resolve();

    expect(cacheDump()).not.toContain("secret-from-a");
    expect(cacheDump()).not.toContain("late-secret-from-a");
    expect(
      queryClient.getQueryData(
        dmKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();

    await expect(
      queryClient.fetchQuery({
        queryKey: serverKeys.list(bob.id, takeStamp()?.generation ?? 0),
        queryFn: () => Promise.reject(new Error("down")),
        retry: false,
      }),
    ).rejects.toThrow("down");
    expect(
      queryClient.getQueryData(
        serverKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();
    expect(cacheDump()).not.toContain("secret-server-a");
    expect(useMediaSettings.getState().audioInputId).toBe("mic-a");
    expect(useTheme.getState().preference).toBe("dark");
  });

  it("a 401 ends the session the same way as logout", async () => {
    await login(ada.email, "password123");
    const generation = takeStamp()?.generation ?? 0;
    queryClient.setQueryData(dmKeys.list(ada.id, generation), [secretDm]);

    await expect(api("/servers")).rejects.toMatchObject({
      code: "unauthenticated",
    });

    expect(cacheDump()).not.toContain("secret-from-a");

    who = bob;
    await login(bob.email, "password123");
    expect(
      queryClient.getQueryData(
        dmKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();
    expect(cacheDump()).not.toContain("secret-from-a");
  });

  it("a late response does not refill the same account after re-login", async () => {
    await login(ada.email, "password123");
    const first = takeStamp();
    expect(first).toBeDefined();
    let release: ((rows: DirectMessage[]) => void) | undefined;
    const flight = queryClient.fetchQuery({
      queryKey: dmKeys.list(ada.id, first?.generation ?? 0),
      retry: false,
      queryFn: () =>
        new Promise<DirectMessage[]>((resolve) => {
          release = resolve;
        }),
    });

    await logout();
    await login(ada.email, "password123");
    const second = takeStamp();
    expect(second?.generation).not.toBe(first?.generation);

    release?.([secretDm]);
    await flight.catch(() => undefined);
    cacheOpenedDm(queryClient, first, secretDm);
    await Promise.resolve();
    await Promise.resolve();

    expect(
      queryClient.getQueryData(dmKeys.list(ada.id, second?.generation ?? 0)),
    ).toBeUndefined();
    expect(cacheDump()).not.toContain("secret-from-a");
  });

  it("a profile update for the same account keeps the cache", async () => {
    await login(ada.email, "password123");
    const generation = takeStamp()?.generation ?? 0;
    queryClient.setQueryData(dmKeys.list(ada.id, generation), [secretDm]);

    await updateProfile({ name: "Ada Lovelace" });

    expect(queryClient.getQueryData(dmKeys.list(ada.id, generation))).toEqual([
      secretDm,
    ]);
    expect(takeStamp()?.generation).toBe(generation);
  });

  it("a late profile response does not put the previous account back", async () => {
    let resolvePatch: ((response: Response) => void) | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const key = `${init?.method ?? "GET"} ${String(input)}`;
        if (key === "POST /api/auth/login") {
          return Promise.resolve(json(200, { user: who, csrf_token: "csrf" }));
        }
        if (key === "POST /api/auth/logout") {
          return Promise.resolve(json(200, { csrf_token: "after-logout" }));
        }
        if (key === "PATCH /api/me") {
          return new Promise<Response>((resolve) => {
            resolvePatch = resolve;
          });
        }
        return Promise.reject(new Error(`unexpected request ${key}`));
      }),
    );

    who = ada;
    await login(ada.email, "password123");
    const pending = updateProfile({ name: "Ada Lovelace" });
    await logout();
    who = bob;
    await login(bob.email, "password123");
    const bobGeneration = takeStamp()?.generation;

    resolvePatch?.(json(200, { ...ada, name: "Ada Lovelace" }));
    await pending;

    expect(useSession.getState().user?.id).toBe(bob.id);
    expect(takeStamp()).toEqual({ userId: bob.id, generation: bobGeneration });
    expect(cacheDump()).not.toContain("secret-from-a");
  });

  it("a late profile error does not roll back onto the next account", async () => {
    let resolvePatch: ((response: Response) => void) | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const key = `${init?.method ?? "GET"} ${String(input)}`;
        if (key === "POST /api/auth/login") {
          return Promise.resolve(json(200, { user: who, csrf_token: "csrf" }));
        }
        if (key === "POST /api/auth/logout") {
          return Promise.resolve(json(200, { csrf_token: "after-logout" }));
        }
        if (key === "PATCH /api/me") {
          return new Promise<Response>((resolve) => {
            resolvePatch = resolve;
          });
        }
        return Promise.reject(new Error(`unexpected request ${key}`));
      }),
    );

    who = ada;
    await login(ada.email, "password123");
    const pending = updateProfile({ name: "Ada Lovelace" });
    expect(useSession.getState().user?.name).toBe("Ada Lovelace");
    await logout();
    who = bob;
    await login(bob.email, "password123");

    resolvePatch?.(
      json(422, {
        error: "validation_failed",
        message: "x",
        fields: { name: "invalid" },
      }),
    );
    await expect(pending).rejects.toMatchObject({ code: "validation_failed" });
    expect(useSession.getState().user).toEqual(bob);
  });

  it("a profile response for a different user goes through the session boundary", async () => {
    await login(ada.email, "password123");
    const generation = takeStamp()?.generation ?? 0;
    queryClient.setQueryData(dmKeys.list(ada.id, generation), [secretDm]);
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const key = `${init?.method ?? "GET"} ${String(input)}`;
        if (key === "PATCH /api/me") {
          return Promise.resolve(json(200, bob));
        }
        return Promise.reject(new Error(`unexpected request ${key}`));
      }),
    );

    await updateProfile({ name: "Bob" });

    expect(useSession.getState().user).toEqual(bob);
    expect(takeStamp()?.userId).toBe(bob.id);
    expect(takeStamp()?.generation).not.toBe(generation);
    expect(cacheDump()).not.toContain("secret-from-a");
    expect(
      queryClient.getQueryData(
        dmKeys.list(bob.id, takeStamp()?.generation ?? 0),
      ),
    ).toBeUndefined();
  });
});
