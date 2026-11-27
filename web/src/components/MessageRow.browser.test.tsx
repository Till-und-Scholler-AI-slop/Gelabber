import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";

const clipboard = vi.hoisted(() => ({
  copyText: vi.fn<(text: string) => Promise<boolean>>(async () => true),
}));
vi.mock("../servers/clipboard.ts", () => clipboard);

import { useSession } from "../auth/store.ts";
import { resetReplyTargets, useReplyTargets } from "../messages/replyTarget.ts";
import type { Message } from "../messages/types.ts";
import { resetEditDrafts } from "./editDrafts.ts";
import { MessageRow } from "./MessageRow.tsx";

const message: Message = {
  id: "m1",
  channel_id: "c1",
  author: { id: "u1", name: "Ada", avatar_url: null },
  content: "Treffen wir uns um acht?",
  created_at: "2026-11-27T18:00:00Z",
  edited_at: null,
  attachments: [],
};

beforeEach(() => {
  clipboard.copyText.mockClear();
  // Both outlive the row, as they do when it scrolls out of view.
  resetReplyTargets();
  resetEditDrafts();
  useSession.setState({
    status: "authenticated",
    user: {
      id: "u1",
      email: "ada@example.com",
      name: "Ada",
      avatar_url: null,
      created_at: "2026-01-01T00:00:00.000Z",
      locale: "de",
    },
  });
});

async function renderRow() {
  const onDelete = vi.fn();
  const screen = await render(
    <QueryClientProvider client={new QueryClient()}>
      <MessageRow
        message={message}
        continued={false}
        mine
        canSend
        canDelete
        onEdit={() => {}}
        onDelete={onDelete}
      />
    </QueryClientProvider>,
  );
  const more = screen.getByRole("button", { name: "Weitere Aktionen" });
  const menu = page.getByRole("menu", { name: "Nachricht von Ada" });
  return { screen, more, menu, onDelete };
}

describe("message menu", () => {
  it("opens at „Weitere Aktionen“ with focus inside, moves by arrow keys, and Esc hands focus back", async () => {
    const { more, menu } = await renderRow();
    await userEvent.click(more);
    await expect.element(menu).toBeVisible();
    await expect.element(more).toHaveAttribute("aria-expanded", "true");
    const entry = (name: string) =>
      menu.getByRole("menuitem", { name }).element();
    expect(document.activeElement).toBe(entry("Mit 👍 reagieren"));

    await userEvent.keyboard("{End}");
    expect(document.activeElement).toBe(entry("Löschen"));
    // Round from the last entry to the first.
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(entry("Mit 👍 reagieren"));
    await userEvent.keyboard("{ArrowUp}{ArrowUp}");
    expect(document.activeElement).toBe(entry("Text kopieren"));

    await userEvent.keyboard("{Escape}");
    await expect.element(menu).not.toBeInTheDocument();
    expect(document.activeElement).toBe(more.element());
  });

  it("opens by a right click on the message", async () => {
    const { screen, menu } = await renderRow();
    await userEvent.click(screen.getByText("Treffen wir uns um acht?"), {
      button: "right",
    });
    await expect.element(menu).toBeVisible();
  });

  it("answers the message from „Antworten“ and closes", async () => {
    const { more, menu } = await renderRow();
    await userEvent.click(more);
    await userEvent.click(menu.getByRole("menuitem", { name: "Antworten" }));
    await expect.element(menu).not.toBeInTheDocument();
    expect(useReplyTargets.getState().byChannel.c1?.id).toBe("m1");
  });

  it("edits the message in place from „Bearbeiten“", async () => {
    const { screen, more, menu } = await renderRow();
    await userEvent.click(more);
    await userEvent.click(menu.getByRole("menuitem", { name: "Bearbeiten" }));
    const editor = screen.getByRole("combobox");
    await expect.element(editor).toHaveValue("Treffen wir uns um acht?");
    expect(document.activeElement).toBe(editor.element());
  });

  it("copies the message's text from „Text kopieren“", async () => {
    const { more, menu } = await renderRow();
    await userEvent.click(more);
    await userEvent.click(
      menu.getByRole("menuitem", { name: "Text kopieren" }),
    );
    expect(clipboard.copyText).toHaveBeenCalledExactlyOnceWith(
      "Treffen wir uns um acht?",
    );
    expect(useReplyTargets.getState().byChannel.c1).toBeUndefined();
  });

  it("deletes with the confirmation left on without Shift", async () => {
    const { more, menu, onDelete } = await renderRow();
    await userEvent.click(more);
    await userEvent.click(menu.getByRole("menuitem", { name: "Löschen" }));
    expect(onDelete).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("deletes with the confirmation skipped when Shift is held", async () => {
    const { more, menu, onDelete } = await renderRow();
    await userEvent.click(more);
    await userEvent.keyboard("{Shift>}");
    await userEvent.click(menu.getByRole("menuitem", { name: "Löschen" }));
    await userEvent.keyboard("{/Shift}");
    expect(onDelete).toHaveBeenCalledExactlyOnceWith(true);
  });
});
