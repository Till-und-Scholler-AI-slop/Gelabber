import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  status: undefined as { enabled: boolean; unread: number } | undefined,
  pending: false,
  sending: false,
  api: vi.fn(),
  dialog: {
    open: true,
    generation: 0,
    draft: {
      kind: "idea" as "idea" | "problem" | "praise",
      text: "",
      image: null as File | null,
      preview: null as string | null,
    },
  },
}));

// A server render reads a store's initial state: the dialog's comes from here.
vi.mock("./dialogStore.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./dialogStore.ts")>()),
  useFeedbackDialog: (select: (state: typeof fixture.dialog) => unknown) =>
    select(fixture.dialog),
}));

vi.mock("./queries.ts", () => ({
  useFeedbackStatus: () => ({ data: fixture.status }),
  useSendFeedback: () => ({ isPending: fixture.pending, mutate: vi.fn() }),
  useFeedbackSending: () => fixture.sending,
}));
vi.mock("../api/client.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api/client.ts")>()),
  api: fixture.api,
}));
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  Link: ({
    to,
    params,
    children,
  }: {
    to: string;
    params?: Record<string, string>;
    children?: ReactNode;
  }) => (
    <a href={to.replace(/\$(\w+)/g, (_, name: string) => params?.[name] ?? "")}>
      {children}
    </a>
  ),
  useNavigate: () => () => Promise.resolve(),
}));

import { ApiError } from "../api/client.ts";
import { restoreFocus } from "../components/modalClose.ts";
import { uploadTimeoutMs } from "../messages/api.ts";
import { sendFeedback } from "./api.ts";
import { EMPTY_DRAFT } from "./dialogStore.ts";
import { FeedbackDialog } from "./FeedbackDialog.tsx";
import {
  IMAGE_MAX_BYTES,
  charactersLeft,
  clipboardImage,
  imageProblem,
  sendFieldErrors,
} from "./rules.ts";

const context = {
  client: "web" as const,
  os: "linux" as const,
  app_version: "0.10.0",
  locale: "de" as const,
  page: "/s/home",
};

function reset() {
  fixture.dialog = { open: true, generation: 0, draft: { ...EMPTY_DRAFT } };
}

// What is read out, and the typing and sending, are in
// FeedbackDialog.browser.test.tsx.
describe("the dialog", () => {
  beforeEach(reset);
  const updateFeedbackDraft = (patch: Partial<typeof fixture.dialog.draft>) => {
    fixture.dialog.draft = { ...fixture.dialog.draft, ...patch };
  };
  afterEach(() => {
    reset();
    fixture.status = undefined;
    fixture.pending = false;
  });
  const html = () => renderToStaticMarkup(<FeedbackDialog />);

  it("offers an image with its hint, and shows an attached one", () => {
    let markup = html();
    expect(markup).toContain(
      '<input type="file" accept="image/png,image/jpeg,image/webp" hidden="" tabindex="-1"/>',
    );
    expect(markup).toContain('aria-describedby="feedback-image-hint"');
    expect(markup).toContain(
      "PNG, JPEG oder WebP, höchstens 5 MB. Du kannst ein Bild auch einfügen.",
    );
    updateFeedbackDraft({
      image: new File(["png"], "shot.png", { type: "image/png" }),
      preview: "blob:shot",
    });
    markup = html();
    expect(markup).toMatch(/<img src="blob:shot" alt="Angehängtes Bild"\/>/);
    expect(markup).toContain('aria-label="Bild entfernen"');
  });

  it("links a new reply only while there is one", () => {
    fixture.status = { enabled: true, unread: 0 };
    expect(html()).not.toContain("neue Antwort");
    fixture.status = { enabled: true, unread: 1 };
    expect(html()).toContain(
      'Es gibt eine neue Antwort auf dein Feedback. <a href="/settings/feedback">Ansehen</a>',
    );
  });

  it("keeps the fields while sending, and says so", () => {
    updateFeedbackDraft({ text: "Hallo" });
    fixture.pending = true;
    const markup = html();
    expect(markup).toMatch(/<textarea[^>]* readOnly=""/);
    expect(markup).toContain("Wird gesendet…");
    expect(markup).toMatch(/role="radio" aria-checked="true"[^>]*disabled=""/);
  });

  it("has Abbrechen and Senden", () => {
    const markup = html();
    expect(markup).toContain(">Abbrechen</button>");
    expect(markup).toMatch(/<button type="submit"[^>]*>Senden<\/button>/);
  });
});

describe("closing", () => {
  it("hands focus back to the user menu's chevron", () => {
    // The menu item focuses the chevron before it opens the dialog, so the
    // chevron is what the dialog saw as its opener; the menu is closed.
    const chevron = {
      isConnected: true,
      getClientRects: () => ({ length: 1 }),
      focus: vi.fn(),
      closest: () => ({ querySelector: () => chevron }),
    };
    expect(restoreFocus(chevron)).toBe(true);
    expect(chevron.focus).toHaveBeenCalledOnce();
  });
});

describe("the dialog's rules", () => {
  it("counts down from 3,600 characters", () => {
    expect(charactersLeft("a".repeat(3599))).toBeNull();
    expect(charactersLeft("a".repeat(3600))).toBe(400);
    expect(charactersLeft("a".repeat(4000))).toBe(0);
  });

  it("refuses other types and images over 5 MB", () => {
    expect(imageProblem({ type: "image/gif", size: 10 })).toBe("wrongType");
    expect(imageProblem({ type: "image/svg+xml", size: 10 })).toBe("wrongType");
    expect(imageProblem({ type: "image/png", size: IMAGE_MAX_BYTES + 1 })).toBe(
      "tooLarge",
    );
    for (const type of ["image/png", "image/jpeg", "image/webp"])
      expect(imageProblem({ type, size: IMAGE_MAX_BYTES })).toBeNull();
  });

  it("takes the clipboard's image and leaves its text alone", () => {
    const image = new File(["x"], "paste.png", { type: "image/png" });
    const text = { kind: "string", type: "text/plain", getAsFile: () => null };
    const file = { kind: "file", type: "image/png", getAsFile: () => image };
    expect(clipboardImage([text, file])).toBe(image);
    expect(clipboardImage([text])).toBeNull();
    expect(clipboardImage(undefined)).toBeNull();
  });

  it("puts the API's refusals under their fields", () => {
    const refused = (fields: Record<string, string>) =>
      new ApiError(
        "validation_failed",
        422,
        "",
        fields as ConstructorParameters<typeof ApiError>[3],
      );
    expect(sendFieldErrors(refused({ text: "required" }))).toEqual({
      text: "Schreib ein paar Worte.",
    });
    expect(sendFieldErrors(refused({ text: "too_long" }))).toEqual({
      text: "Höchstens 4.000 Zeichen.",
    });
    // A reply to a thread of 200 messages.
    expect(sendFieldErrors(refused({ text: "too_many" }))).toEqual({
      text: "Dieser Verlauf ist voll (200 Nachrichten).",
    });
    expect(sendFieldErrors(refused({ image: "invalid" }))).toEqual({
      image: "Das Bild ließ sich nicht verwenden. Versuch ein anderes.",
    });
    expect(
      sendFieldErrors(new ApiError("internal", 413, "length limit exceeded")),
    ).toEqual({ image: "Das Bild ist größer als 5 MB." });
    // Not a field's: the form's error line says it.
    expect(sendFieldErrors(new ApiError("rate_limited", 429, ""))).toBeNull();
    expect(sendFieldErrors(new ApiError("quota_exceeded", 429, ""))).toBeNull();
    expect(sendFieldErrors(refused({ page: "invalid" }))).toBeNull();
  });
});

describe("sending", () => {
  afterEach(() => fixture.api.mockReset());

  it("uploads the image first, with the upload's timeout, then creates", async () => {
    fixture.api
      .mockResolvedValueOnce({ image: "img-1" })
      .mockResolvedValueOnce({ id: "entry" });
    const image = new File([new Uint8Array(4096)], "a.png", {
      type: "image/png",
    });
    await expect(
      sendFeedback({ kind: "problem", text: "Schwarz", context }, image),
    ).resolves.toEqual({ id: "entry" });
    expect(fixture.api.mock.calls).toEqual([
      [
        "/feedback/images",
        { method: "POST", body: image, timeoutMs: uploadTimeoutMs(4096) },
      ],
      [
        "/feedback",
        {
          method: "POST",
          body: { kind: "problem", text: "Schwarz", context, image: "img-1" },
        },
      ],
    ]);
  });

  it("creates at once without an image", async () => {
    fixture.api.mockResolvedValueOnce({ id: "entry" });
    await sendFeedback({ kind: "idea", text: "Mehr", context }, null);
    expect(fixture.api.mock.calls).toEqual([
      [
        "/feedback",
        { method: "POST", body: { kind: "idea", text: "Mehr", context } },
      ],
    ]);
  });

  it("creates nothing when the upload fails", async () => {
    fixture.api.mockRejectedValueOnce(new ApiError("rate_limited", 429, ""));
    await expect(
      sendFeedback(
        { kind: "idea", text: "Mehr", context },
        new File(["x"], "a.png", { type: "image/png" }),
      ),
    ).rejects.toMatchObject({ code: "rate_limited" });
    expect(fixture.api).toHaveBeenCalledOnce();
  });
});
