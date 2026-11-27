import { beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";

const sending = vi.hoisted(() => ({
  mutate: vi.fn(),
  /** What the dialog runs once the server took the feedback. */
  onSent: undefined as (() => void) | undefined,
}));
vi.mock("./queries.ts", () => ({
  useFeedbackStatus: () => ({ data: { enabled: true, unread: 0 } }),
  useSendFeedback: (onSent: () => void) => {
    sending.onSent = onSent;
    return { isPending: false, mutate: sending.mutate };
  },
  useFeedbackSending: () => false,
}));
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => () => Promise.resolve(),
}));

import { OS_NAMES } from "../auth/devices.ts";
import {
  clearFeedbackDraft,
  closeFeedbackDialog,
  openFeedbackDialog,
  updateFeedbackDraft,
} from "./dialogStore.ts";
import { FeedbackDialog } from "./FeedbackDialog.tsx";
import type { FeedbackContext } from "./types.ts";

beforeEach(() => {
  sending.mutate.mockReset();
  closeFeedbackDialog();
  clearFeedbackDraft();
});

async function renderDialog() {
  const screen = await render(
    <>
      <button type="button" onClick={openFeedbackDialog}>
        Feedback geben
      </button>
      <FeedbackDialog />
    </>,
  );
  const opener = screen.getByRole("button", { name: "Feedback geben" });
  const dialog = page.getByRole("dialog", { name: "Feedback geben" });
  const text = dialog.getByRole("textbox", { name: "Deine Nachricht" });
  return { opener, dialog, text };
}

/** What the one send carried. */
function sent() {
  expect(sending.mutate).toHaveBeenCalledOnce();
  return sending.mutate.mock.calls[0]![0] as {
    kind: string;
    text: string;
    image: File | null;
    context: FeedbackContext;
  };
}

describe("feedback dialog", () => {
  it("opens over the page and hands focus back on Esc, keeping the draft", async () => {
    const { opener, dialog, text } = await renderDialog();
    await userEvent.click(opener);
    await expect.element(dialog).toBeVisible();
    await userEvent.fill(text, "Halbfertig");
    await userEvent.keyboard("{Escape}");
    await expect.element(dialog).not.toBeInTheDocument();
    expect(document.activeElement).toBe(opener.element());

    await userEvent.click(opener);
    await expect.element(text).toHaveValue("Halbfertig");
  });

  it("sends nothing without a text and says why at the field", async () => {
    const { opener, dialog, text } = await renderDialog();
    await userEvent.click(opener);
    await userEvent.click(dialog.getByRole("button", { name: "Senden" }));
    await expect
      .element(text)
      .toHaveAccessibleDescription(/Schreib ein paar Worte\./);
    await expect.element(text).toHaveAttribute("aria-invalid", "true");
    expect(sending.mutate).not.toHaveBeenCalled();
  });

  it("sends the kind chosen by arrow key and the text, then closes empty", async () => {
    const { opener, dialog, text } = await renderDialog();
    await userEvent.click(opener);
    const kinds = dialog.getByRole("radiogroup", { name: "Worum geht es?" });
    await userEvent.click(kinds.getByRole("radio", { name: "Idee" }));
    await userEvent.keyboard("{ArrowRight}");
    await expect
      .element(kinds.getByRole("radio", { name: "Problem" }))
      .toHaveAttribute("aria-checked", "true");
    expect(document.activeElement).toBe(
      kinds.getByRole("radio", { name: "Problem" }).element(),
    );
    await expect
      .element(text)
      .toHaveAttribute(
        "placeholder",
        "Was ist passiert, und was hättest du erwartet?",
      );

    await userEvent.fill(text, "Der Ton bricht ab.");
    await userEvent.click(dialog.getByRole("button", { name: "Senden" }));
    expect(sent()).toMatchObject({
      kind: "problem",
      text: "Der Ton bricht ab.",
      image: null,
    });

    sending.onSent?.();
    await expect.element(dialog).not.toBeInTheDocument();
    await userEvent.click(opener);
    await expect.element(text).toHaveValue("");
  });

  it("attaches a pasted picture and sends it along", async () => {
    const { opener, dialog, text } = await renderDialog();
    await userEvent.click(opener);
    await userEvent.fill(text, "Siehe Bild");
    const data = new DataTransfer();
    data.items.add(
      new File([new Uint8Array([137, 80, 78, 71])], "shot.png", {
        type: "image/png",
      }),
    );
    text.element().dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData: data,
        bubbles: true,
        cancelable: true,
      }),
    );
    await expect
      .element(dialog.getByRole("img", { name: "Angehängtes Bild" }))
      .toBeVisible();
    await userEvent.click(dialog.getByRole("button", { name: "Senden" }));
    expect(sent().image?.name).toBe("shot.png");
  });
});

describe("feedback dialog, as read out", () => {
  async function opened() {
    const parts = await renderDialog();
    await userEvent.click(parts.opener);
    await expect.element(parts.dialog).toBeVisible();
    return parts;
  }

  it("is titled and described", async () => {
    const { dialog } = await opened();
    await expect
      .element(dialog)
      .toHaveAccessibleDescription(
        "Sag den Betreibern dieser Instanz, was dir auffällt.",
      );
  });

  it("offers the three kinds as one radio group, Idee chosen and the one Tab stop", async () => {
    const { dialog } = await opened();
    const kinds = dialog
      .getByRole("radiogroup", { name: "Worum geht es?" })
      .getByRole("radio")
      .elements()
      .map((chip) => [
        chip.textContent,
        chip.getAttribute("aria-checked"),
        chip.tabIndex,
      ]);
    expect(kinds).toEqual([
      ["Idee", "true", 0],
      ["Problem", "false", -1],
      ["Lob", "false", -1],
    ]);
  });

  it("asks for the text with the kind's placeholder and the limits", async () => {
    const { dialog, text } = await opened();
    await expect.element(text).toBeRequired();
    await expect.element(text).toHaveAttribute("maxlength", "4000");
    await expect
      .element(text)
      .toHaveAttribute("placeholder", "Was würdest du dir wünschen?");
    await userEvent.click(dialog.getByRole("radio", { name: "Problem" }));
    await expect
      .element(text)
      .toHaveAttribute(
        "placeholder",
        "Was ist passiert, und was hättest du erwartet?",
      );
    await userEvent.click(dialog.getByRole("radio", { name: "Lob" }));
    await expect
      .element(text)
      .toHaveAttribute("placeholder", "Was gefällt dir?");
  });

  it("counts down from 3,600 characters, announced under 100 left", async () => {
    updateFeedbackDraft({ text: "a".repeat(3599) });
    const { dialog, text } = await opened();
    const counter = dialog.getByText(/^Noch \d+ Zeichen$/);
    await expect.element(counter).not.toBeInTheDocument();

    updateFeedbackDraft({ text: "a".repeat(3600) });
    await expect.element(counter).toHaveTextContent("Noch 400 Zeichen");
    await expect.element(text).toHaveAccessibleDescription(/Noch 400 Zeichen/);
    await expect.element(counter).not.toHaveAttribute("aria-live");

    updateFeedbackDraft({ text: "a".repeat(3950) });
    await expect.element(counter).toHaveTextContent("Noch 50 Zeichen");
    await expect.element(counter).toHaveAttribute("aria-live", "polite");
  });

  it("shows exactly what is sent along", async () => {
    const { dialog, text } = await opened();
    const shown = (role: "term" | "definition") =>
      dialog
        .getByRole(role)
        .elements()
        .map((element) => element.textContent);
    const terms = shown("term");
    const values = shown("definition");
    await userEvent.fill(text, "Hallo");
    await userEvent.click(dialog.getByRole("button", { name: "Senden" }));
    const { context } = sent();

    expect(Object.keys(context).sort()).toEqual([
      "app_version",
      "client",
      "locale",
      "os",
      "page",
    ]);
    expect(context).toMatchObject({ client: "web", locale: "de" });
    expect(terms).toEqual(["App", "System", "Version", "Sprache", "Seite"]);
    expect(values).toEqual([
      "Web-App",
      context.os === "other" ? "Unbekannt" : OS_NAMES[context.os],
      context.app_version,
      "Deutsch",
      context.page,
    ]);
    await expect
      .element(
        dialog.getByText(
          "Sonst nichts: keine Protokolle, keine weiteren Daten.",
        ),
      )
      .toBeVisible();
  });
});
