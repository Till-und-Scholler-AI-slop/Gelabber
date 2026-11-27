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
import { MentionContext } from "./mentionContext.ts";
import { MessageRow } from "./MessageRow.tsx";

const message: Message = {
  id: "m1",
  channel_id: "c1",
  author: { id: "u1", name: "Ada", avatar_url: null },
  content: "Treffen wir uns um acht?",
  created_at: "2026-11-27T18:00:00Z",
  edited_at: null,
  attachments: [],
  reactions: [],
  mentions: [],
  mention_everyone: false,
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

describe("a row that cannot be drawn", () => {
  // A message that slipped past the checks without `mentions`: drawing it
  // for another reader throws.
  const broken = {
    ...message,
    id: "m2",
    revision: 1,
    author: { id: "u2", name: "Bea", avatar_url: null },
    content: "Kaputt",
    mentions: undefined,
  } as unknown as Message;
  const row = (shown: Message) => (
    <MessageRow
      key={shown.id}
      message={shown}
      continued={false}
      mine={shown.author.id === "u1"}
      canSend
      canDelete={false}
      onEdit={() => {}}
      onDelete={() => {}}
    />
  );
  const rows = (...shown: Message[]) => (
    <QueryClientProvider client={new QueryClient()}>
      <MentionContext.Provider
        value={{ members: [], mentionable: [], me: "u1", keywords: false }}
      >
        {shown.map(row)}
      </MentionContext.Provider>
    </QueryClientProvider>
  );
  const notice = "Diese Nachricht kann nicht angezeigt werden.";

  it("shows a notice in its place, and the row beside it stays", async () => {
    const screen = await render(rows(message, broken));
    await expect
      .element(screen.getByText("Treffen wir uns um acht?"))
      .toBeVisible();
    const failed = screen.getByText(notice);
    await expect.element(failed).toBeVisible();
    // Who wrote it and when, as far as they are readable.
    await expect.element(screen.getByText(/^Bea · \d/)).toBeVisible();
    await expect
      .element(screen.getByRole("button", { name: "Erneut versuchen" }))
      .toBeVisible();
    await expect.element(screen.getByText("Kaputt")).not.toBeInTheDocument();
    // Focusable like a row, and not announced again each time it mounts.
    const box = screen.container.querySelector(".lr-message-failed");
    expect(box?.getAttribute("tabindex")).toBe("-1");
    expect(box?.getAttribute("role")).toBeNull();
  });

  it("draws the message again once a newer revision of it arrives", async () => {
    const screen = await render(rows(broken));
    await expect.element(screen.getByText(notice)).toBeVisible();
    await screen.rerender(
      rows({
        ...message,
        id: "m2",
        revision: 2,
        author: { id: "u2", name: "Bea", avatar_url: null },
        content: "Wieder da",
      }),
    );
    await expect.element(screen.getByText("Wieder da")).toBeVisible();
    await expect.element(screen.getByText(notice)).not.toBeInTheDocument();
  });
});
