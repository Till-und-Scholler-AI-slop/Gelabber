import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { login, resetSessionForTests } from "../auth/session.ts";
import { previewText } from "../messages/notify.ts";
import {
  resetPendingMessages,
  usePendingMessages,
} from "../messages/pending.ts";
import { sendMessageAttempt } from "../messages/queries.ts";
import { asGif, type Gif } from "../messages/types.ts";
import { gifBox, gifLabel, gifMediaUrl, gifPage } from "./api.ts";

const gif: Gif = {
  provider: "klipy",
  slug: "hello-hi-662",
  title: "Hello",
  width: 498,
  height: 249,
  url: "https://static.klipy.com/ii/a/b.webp?x=1&y=2",
  still: "https://static.klipy.com/ii/a/b.jpg",
};

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

const user = {
  id: "a",
  name: "Ada",
  email: "a@example.test",
  avatar_url: null,
  created_at: "2026-09-28T00:00:00Z",
};
const author = { id: user.id, name: user.name, avatar_url: null };

let calls: { key: string; body: unknown }[];
function routes(handler: (key: string) => Response) {
  vi.stubGlobal(
    "fetch",
    vi.fn((url: RequestInfo | URL, init: RequestInit = {}) => {
      const key = `${init.method ?? "GET"} ${String(url)}`;
      calls.push({
        key,
        body: init.body ? JSON.parse(String(init.body)) : undefined,
      });
      return Promise.resolve(
        key === "POST /api/auth/login"
          ? json(200, { user, csrf_token: "csrf" })
          : handler(key),
      );
    }),
  );
}

beforeEach(() => {
  calls = [];
  resetSessionForTests();
  resetPendingMessages();
});
afterEach(() => {
  resetSessionForTests();
  resetPendingMessages();
  vi.unstubAllGlobals();
});

describe("gif helpers", () => {
  it("loads media only through the relay", () => {
    const relayed = gifMediaUrl(gif.url);
    expect(relayed.startsWith("/api/gifs/media?url=")).toBe(true);
    expect(new URL(relayed, "http://x").searchParams.get("url")).toBe(gif.url);
  });

  it("fits a box of at most 320 × 240 without upscaling", () => {
    expect(gifBox(gif)).toEqual({ width: 320, height: 160 });
    expect(gifBox({ width: 100, height: 400 })).toEqual({
      width: 60,
      height: 240,
    });
    expect(gifBox({ width: 80, height: 60 })).toEqual({
      width: 80,
      height: 60,
    });
  });

  it("labels by title", () => {
    expect(gifLabel(gif)).toBe("GIF: Hello");
    expect(gifLabel({ title: "" })).toBe("GIF");
  });

  it("accepts only whole GIFs from live events", () => {
    expect(asGif(gif)).toEqual(gif);
    expect(asGif({ ...gif, still: null, title: undefined })).toEqual({
      ...gif,
      still: null,
      title: "",
    });
    expect(asGif(null)).toBeNull();
    expect(asGif({ ...gif, width: 0 })).toBeNull();
    expect(asGif({ slug: "x" })).toBeNull();
  });

  it("previews a GIF message as GIF", () => {
    expect(previewText("", false, true)).toBe("GIF");
    expect(previewText("", false)).toBe("Neue Nachricht");
  });

  it("asks trending without a query and search with one", async () => {
    routes(() => json(200, { gifs: [], next_page: null }));
    await login(user.email, "password");
    await gifPage("", 1);
    await gifPage("  hallo welt ", 2);
    expect(calls.slice(1).map((call) => call.key)).toEqual([
      "GET /api/gifs/trending",
      "GET /api/gifs/search?q=hallo+welt&page=2",
    ]);
  });
});

describe("sending a GIF", () => {
  it("shows it at once and posts only the slug and query", async () => {
    routes((key) =>
      key === "POST /api/channels/c/messages"
        ? json(201, {
            id: "server-1",
            channel_id: "c",
            author,
            content: "",
            created_at: "2026-09-28T00:00:00Z",
            edited_at: null,
            attachments: [],
            gif,
          })
        : json(404, { error: "not_found" }),
    );
    await login(user.email, "password");
    const sending = sendMessageAttempt("c", author, {
      content: "draft text stays out",
      gif: { slug: gif.slug, query: "hallo", gif },
    });
    const pending = usePendingMessages.getState().byChannel.c ?? [];
    expect(pending[0]?.gif).toEqual(gif);
    expect(pending[0]?.content).toBe("");
    const message = await sending;
    expect(message.gif?.slug).toBe(gif.slug);
    const post = calls.find(
      (call) => call.key === "POST /api/channels/c/messages",
    );
    expect(post?.body).toEqual({
      content: "",
      gif: { slug: gif.slug, query: "hallo" },
    });
  });

  it("keeps a failed GIF send retryable", async () => {
    let fail = true;
    routes((key) => {
      if (key !== "POST /api/channels/c/messages")
        return json(404, { error: "not_found" });
      if (fail)
        return json(422, {
          error: "validation_failed",
          fields: { gif: "invalid" },
        });
      return json(201, {
        id: "server-2",
        channel_id: "c",
        author,
        content: "",
        created_at: "2026-09-28T00:00:00Z",
        edited_at: null,
        attachments: [],
        gif,
      });
    });
    await login(user.email, "password");
    await expect(
      sendMessageAttempt("c", author, {
        content: "",
        gif: { slug: gif.slug, query: "", gif },
      }),
    ).rejects.toBeTruthy();
    const [attempt] = Object.values(usePendingMessages.getState().attempts);
    expect(attempt?.status).toBe("failed");
    expect(attempt?.gif?.slug).toBe(gif.slug);
    fail = false;
    const message = await sendMessageAttempt("c", author, {
      content: "",
      attemptId: attempt?.id,
    });
    expect(message.id).toBe("server-2");
    const posts = calls.filter(
      (call) => call.key === "POST /api/channels/c/messages",
    );
    expect(posts.at(-1)?.body).toEqual({
      content: "",
      gif: { slug: gif.slug, query: "" },
    });
  });
});
