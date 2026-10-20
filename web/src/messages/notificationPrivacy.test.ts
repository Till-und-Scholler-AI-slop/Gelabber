import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { useSession } from "../auth/session.ts";
import { scopeGeneration } from "../auth/scope.ts";
import { closeMessageNotifications } from "../pwa/notifications.ts";
import {
  hidesContent,
  messageNotificationText,
  notificationText,
  privacyKey,
  refreshNotificationPrivacy,
  type NotificationPrivacy,
} from "./notificationPrivacy.ts";

vi.mock("../pwa/notifications.ts", () => ({
  closeMessageNotifications: vi.fn(() => Promise.resolve()),
}));

const shown = {
  title: "Ada hat dich erwähnt · #allgemein",
  body: "@Bob das Passwort ist hunter2",
  author: "Ada",
};

describe("notificationText", () => {
  it("shows the message unless it is hidden", () => {
    expect(notificationText(false, shown)).toEqual({
      title: shown.title,
      body: shown.body,
    });
  });

  it("says only who wrote when the content is hidden", () => {
    expect(notificationText(true, shown)).toEqual({
      title: "Neue Nachricht von Ada",
      body: "",
    });
  });
});

describe("hidesContent", () => {
  it("follows the account's switch once it is known", () => {
    expect(
      hidesContent({
        data: { hide_content: true },
        error: null,
        isInvalidated: false,
      }),
    ).toBe(true);
    expect(
      hidesContent({
        data: { hide_content: false },
        error: null,
        isInvalidated: false,
      }),
    ).toBe(false);
  });

  it("hides while the switch is not known yet", () => {
    expect(hidesContent(undefined)).toBe(true);
    expect(
      hidesContent({ data: undefined, error: null, isInvalidated: false }),
    ).toBe(true);
  });

  it("hides while another device's change is being fetched", () => {
    expect(
      hidesContent({
        data: { hide_content: false },
        error: null,
        isInvalidated: true,
      }),
    ).toBe(true);
  });

  it("hides when the last fetch failed, even with an older answer", () => {
    const offline = new ApiError("network", 0, "offline");
    expect(
      hidesContent({ data: undefined, error: offline, isInvalidated: true }),
    ).toBe(true);
    expect(
      hidesContent({
        data: { hide_content: false },
        error: offline,
        isInvalidated: true,
      }),
    ).toBe(true);
    const busy = new ApiError("internal", 500, "busy");
    expect(
      hidesContent({ data: undefined, error: busy, isInvalidated: true }),
    ).toBe(true);
  });

  it("shows the message on a server from before the switch", () => {
    const missing = new ApiError("not_found", 404, "not found");
    expect(
      hidesContent({ data: undefined, error: missing, isInvalidated: true }),
    ).toBe(false);
  });
});

describe("refreshNotificationPrivacy", () => {
  afterEach(() => {
    useSession.setState({ status: "anonymous", user: null });
    vi.mocked(closeMessageNotifications).mockClear();
  });

  async function refreshed(before: boolean, after: boolean) {
    useSession.setState({
      status: "authenticated",
      user: {
        id: "u1",
        email: "ada@example.com",
        name: "Ada",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00.000Z",
      },
    });
    const stamp = { userId: "u1", generation: scopeGeneration() };
    const client = new QueryClient();
    client.setQueryData<NotificationPrivacy>(privacyKey(stamp), {
      hide_content: before,
    });
    // An active query, as on a page that shows notifications.
    const stop = new QueryObserver(client, {
      queryKey: privacyKey(stamp),
      queryFn: () => ({ hide_content: after }),
      staleTime: Infinity,
    }).subscribe(() => {});
    await refreshNotificationPrivacy(client, stamp);
    stop();
    return client.getQueryData<NotificationPrivacy>(privacyKey(stamp));
  }

  it("takes down what this device shows once another one turns it on", async () => {
    expect(await refreshed(false, true)).toEqual({ hide_content: true });
    expect(closeMessageNotifications).toHaveBeenCalledWith({ user: "u1" });
  });

  it("leaves notifications alone when nothing turned it on", async () => {
    await refreshed(true, true);
    await refreshed(true, false);
    await refreshed(false, false);
    expect(closeMessageNotifications).not.toHaveBeenCalled();
  });
});

// The text `useMessageToasts` gives the page's own notification, read from
// the query cache as the hook does.
describe("messageNotificationText", () => {
  const stamp = { userId: "u1", generation: 0 };
  const message = {
    author: "Ada",
    mentioned: true,
    label: "#allgemein",
    preview: "@Bob das Passwort ist hunter2",
  };
  const hidden = { title: "Neue Nachricht von Ada", body: "" };

  it("hides the message while the switch is not loaded yet", () => {
    expect(messageNotificationText(new QueryClient(), stamp, message)).toEqual(
      hidden,
    );
  });

  it("shows the full text when the account does not hide it", () => {
    const client = new QueryClient();
    client.setQueryData<NotificationPrivacy>(privacyKey(stamp), {
      hide_content: false,
    });
    expect(messageNotificationText(client, stamp, message)).toEqual({
      title: "Ada hat dich erwähnt · #allgemein",
      body: "@Bob das Passwort ist hunter2",
    });
  });

  it("hides the message when the account hides it", () => {
    const client = new QueryClient();
    client.setQueryData<NotificationPrivacy>(privacyKey(stamp), {
      hide_content: true,
    });
    expect(messageNotificationText(client, stamp, message)).toEqual(hidden);
  });

  it("reads the switch of the account the notification is for", () => {
    const client = new QueryClient();
    client.setQueryData<NotificationPrivacy>(
      privacyKey({ userId: "someone-else", generation: 0 }),
      { hide_content: false },
    );
    expect(messageNotificationText(client, stamp, message)).toEqual(hidden);
  });
});
