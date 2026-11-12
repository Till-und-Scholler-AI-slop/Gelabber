import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  confirm: vi.fn<
    (request: {
      title: string;
      message?: string;
      confirmLabel: string;
      tone?: string;
    }) => Promise<boolean>
  >(async () => true),
  notify: vi.fn(),
  notifyError: vi.fn(),
  api: {
    markFeedbackDone: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    reopenFeedback: vi.fn<(id: string) => Promise<unknown>>(async () => ({})),
    deleteFeedback: vi.fn<(id: string) => Promise<unknown>>(async () => null),
  },
}));
vi.mock("../components/confirm.ts", () => ({ confirmAction: fixture.confirm }));
vi.mock("../components/toasts.ts", () => ({
  notify: fixture.notify,
  notifyError: fixture.notifyError,
}));
vi.mock("./api.ts", () => fixture.api);

import {
  authorName,
  deleteFeedback,
  feedbackMenuEntries,
  feedbackQuery,
  replyNote,
  senderEmail,
  senderName,
  setFeedbackDone,
  technicalDetails,
} from "./feedbackActions.ts";
import type { FeedbackSender, OperatorFeedback } from "./types.ts";

const account: FeedbackSender = {
  type: "account",
  id: "tom",
  name: "Tom",
  email: "tom@example.org",
  avatar_url: null,
};
const visitor: FeedbackSender = {
  type: "visitor",
  email: "ada@example.org",
  locale: "de",
};
const deleted: FeedbackSender = { type: "deleted" };

beforeEach(() => {
  vi.clearAllMocks();
  fixture.confirm.mockImplementation(async () => true);
});
afterEach(() => setLocale("de", { persist: false }));

describe("feedback filters", () => {
  it("maps status, kind and offset to the list's query", () => {
    expect(feedbackQuery("open", "all", 0)).toEqual({
      status: "open",
      kind: undefined,
      offset: undefined,
    });
    expect(feedbackQuery("done", "problem", 50)).toEqual({
      status: "done",
      kind: "problem",
      offset: 50,
    });
  });
});

describe("feedback senders", () => {
  it("names an account, a visitor and a deleted account", () => {
    expect(senderName({ sender: account })).toBe("Tom");
    expect(senderEmail({ sender: account })).toBe("tom@example.org");
    expect(senderName({ sender: visitor })).toBe("Besucher");
    expect(senderEmail({ sender: visitor })).toBe("ada@example.org");
    expect(senderName({ sender: deleted })).toBe("Gelöschtes Konto");
    expect(senderEmail({ sender: deleted })).toBeNull();
    setLocale("en", { persist: false });
    expect(senderName({ sender: visitor })).toBe("Visitor");
    expect(senderName({ sender: deleted })).toBe("Deleted account");
  });

  it("names the authors of a thread", () => {
    const lena = { id: "lena", name: "Lena" };
    expect(
      authorName({ sender: account }, { from: "operator", author: lena }),
    ).toBe("Lena");
    expect(
      authorName(
        { sender: account },
        { from: "person", author: { id: "tom", name: "Tom" } },
      ),
    ).toBe("Tom");
    expect(
      authorName({ sender: visitor }, { from: "person", author: null }),
    ).toBe("Besucher");
    // An operator's or a sender's account that is gone.
    expect(
      authorName({ sender: account }, { from: "operator", author: null }),
    ).toBe("Gelöschtes Konto");
    expect(
      authorName({ sender: deleted }, { from: "person", author: null }),
    ).toBe("Gelöschtes Konto");
  });
});

describe("the reply note", () => {
  it("tells where an account reads the reply, and the mail when it is on", () => {
    expect(replyNote({ sender: account }, false)).toEqual({
      form: true,
      note: "Tom sieht die Antwort unter Einstellungen → Feedback.",
    });
    expect(replyNote({ sender: account }, true)).toEqual({
      form: true,
      note: "Tom sieht die Antwort unter Einstellungen → Feedback und bekommt eine Mail, dass es eine Antwort gibt.",
    });
  });

  it("mails a visitor, and offers no form without account mail", () => {
    expect(replyNote({ sender: visitor }, true)).toEqual({
      form: true,
      note: "Geht per Mail an ada@example.org, mit deinem Text als Zitat. Antworten per Mail sind nicht möglich.",
    });
    expect(replyNote({ sender: visitor }, false)).toEqual({
      form: false,
      note: "Ohne Account-Mail erreicht eine Antwort diese Person nicht.",
    });
  });

  it("has no note for a deleted account, in English too", () => {
    expect(replyNote({ sender: deleted }, true)).toEqual({
      form: true,
      note: null,
    });
    setLocale("en", { persist: false });
    expect(replyNote({ sender: account }, true).note).toBe(
      "Tom sees the reply under Settings → Feedback and gets an email that there is one.",
    );
    expect(replyNote({ sender: visitor }, true).note).toBe(
      "Goes by email to ada@example.org, quoting your text. They cannot reply by email.",
    );
  });
});

describe("technical details", () => {
  const rows = (context: OperatorFeedback["context"]) =>
    technicalDetails(context).map(({ label, value }) => [label, value]);

  it("lists app, system, version, language and page", () => {
    expect(
      rows({
        client: "desktop",
        os: "windows",
        app_version: "0.10.0",
        locale: "en",
        page: "/settings/darstellung",
      }),
    ).toEqual([
      ["App", "Desktop-App"],
      ["System", "Windows"],
      ["Version", "0.10.0"],
      ["Sprache", "English"],
      ["Seite", "/settings/darstellung"],
    ]);
    expect(
      rows({
        client: "web",
        os: "other",
        app_version: "0.10.0",
        locale: "de",
        page: "/",
      })[1],
    ).toEqual(["System", "Unbekannt"]);
  });

  it("leaves out what the public form does not send", () => {
    expect(
      rows({
        client: "form",
        os: null,
        app_version: null,
        locale: "de",
        page: null,
      }),
    ).toEqual([
      ["App", "Formular auf der Website"],
      ["System", null],
      ["Version", null],
      ["Sprache", "Deutsch"],
      ["Seite", null],
    ]);
  });
});

describe("feedback actions", () => {
  it("marks done and reopens through their routes, then refetches", async () => {
    const refresh = vi.fn();
    expect(await setFeedbackDone({ id: "f1" }, true, refresh)).toBe(true);
    expect(fixture.api.markFeedbackDone).toHaveBeenCalledWith("f1");
    expect(fixture.notify).toHaveBeenLastCalledWith("Als erledigt markiert.");
    expect(await setFeedbackDone({ id: "f1" }, false, refresh)).toBe(true);
    expect(fixture.api.reopenFeedback).toHaveBeenCalledWith("f1");
    expect(fixture.notify).toHaveBeenLastCalledWith("Wieder offen.");
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("says when the entry is gone", async () => {
    const refresh = vi.fn();
    const gone = new ApiError("not_found", 404, "gone");
    fixture.api.markFeedbackDone.mockRejectedValueOnce(gone);
    expect(await setFeedbackDone({ id: "f1" }, true, refresh)).toBe(false);
    expect(fixture.notifyError).toHaveBeenCalledWith(
      gone,
      undefined,
      "Dieses Feedback gibt es nicht mehr.",
    );
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("deletes only after the question", async () => {
    const refresh = vi.fn();
    const entries = feedbackMenuEntries({ delete: vi.fn() });
    expect(entries).toMatchObject([
      { kind: "item", label: "Löschen…", danger: true },
    ]);

    fixture.confirm.mockResolvedValueOnce(false);
    expect(await deleteFeedback({ id: "f1" }, refresh)).toBe(false);
    expect(fixture.api.deleteFeedback).not.toHaveBeenCalled();

    expect(await deleteFeedback({ id: "f1" }, refresh)).toBe(true);
    expect(fixture.confirm).toHaveBeenLastCalledWith({
      title: "Feedback löschen?",
      message:
        "Text, Bild und Antworten sind dann weg. Das lässt sich nicht rückgängig machen.",
      confirmLabel: "Löschen",
      tone: undefined,
    });
    expect(fixture.api.deleteFeedback).toHaveBeenCalledWith("f1");
    expect(fixture.notify).toHaveBeenLastCalledWith("Feedback gelöscht.");
    expect(refresh).toHaveBeenCalledOnce();
  });
});
