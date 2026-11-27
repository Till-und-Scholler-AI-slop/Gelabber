import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";

const sent = vi.hoisted(() => ({ mutate: vi.fn() }));
vi.mock("../messages/queries.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../messages/queries.ts")>()),
  useSendMessage: () => sent,
}));
vi.mock("../gifs/queries.ts", () => ({
  useGifStatus: () => ({ data: { enabled: false } }),
}));

import { useSession } from "../auth/store.ts";
import { Composer } from "./Composer.tsx";

let channel = 0;
beforeEach(() => {
  sent.mutate.mockReset();
  // Drafts are kept per signed-in account.
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
  // A channel of its own per test: drafts outlive the component.
  channel += 1;
});

async function renderComposer() {
  const onSent = vi.fn();
  const screen = await render(
    <QueryClientProvider client={new QueryClient()}>
      <div className="lr-message-pane">
        <Composer
          handle={null}
          channelId={`c${channel}`}
          channelName="allgemein"
          canSend
          canSendFiles
          mention="#"
          author={{ id: "u1", name: "Ada", avatar_url: null }}
          attempts={[]}
          onSent={onSent}
          onShowAttempt={() => {}}
        />
      </div>
    </QueryClientProvider>,
  );
  const field = screen.getByRole("combobox", {
    name: "Nachricht an #allgemein",
  });
  return { screen, field, onSent };
}

const picture = (name: string) =>
  new File([new Uint8Array([137, 80, 78, 71])], name, { type: "image/png" });

function carrying(files: File[]): DataTransfer {
  const data = new DataTransfer();
  for (const file of files) data.items.add(file);
  return data;
}

/** The files the one send carried, by name. */
function sentFiles(): string[] {
  expect(sent.mutate).toHaveBeenCalledOnce();
  const [input] = sent.mutate.mock.calls[0]!;
  return (input as { files: File[] }).files.map((file) => file.name);
}

describe("composer sending", () => {
  it("sends on Enter, keeps Shift+Enter for a new line, and empties the field", async () => {
    const { field, onSent } = await renderComposer();
    await userEvent.click(field);
    await userEvent.keyboard("Hallo{Shift>}{Enter}{/Shift}du");
    expect(sent.mutate).not.toHaveBeenCalled();
    await userEvent.keyboard("{Enter}");

    expect(sent.mutate).toHaveBeenCalledOnce();
    expect(sent.mutate.mock.calls[0]![0]).toMatchObject({
      content: "Hallo\ndu",
      files: [],
    });
    expect(onSent).toHaveBeenCalledOnce();
    await expect.element(field).toHaveValue("");
    // The keyboard stays in the field for the next message.
    expect(document.activeElement).toBe(field.element());
  });

  it("sends nothing while the field is empty or blank", async () => {
    const { screen, field } = await renderComposer();
    await userEvent.click(field);
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard("   {Enter}");
    expect(sent.mutate).not.toHaveBeenCalled();
    await expect
      .element(screen.getByRole("button", { name: "Senden" }))
      .toBeDisabled();
  });

  it("attaches every pasted file and sends them with the text", async () => {
    const { screen, field } = await renderComposer();
    await userEvent.click(field);
    field.element().dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData: carrying([picture("eins.png"), picture("zwei.png")]),
        bubbles: true,
        cancelable: true,
      }),
    );
    const list = screen.getByRole("list", { name: "Anhänge" });
    await expect.element(list.getByText("eins.png")).toBeVisible();
    await expect.element(list.getByText("zwei.png")).toBeVisible();

    await userEvent.keyboard("Zwei Bilder{Enter}");
    expect(sentFiles()).toEqual(["eins.png", "zwei.png"]);
    expect(sent.mutate.mock.calls[0]![0]).toMatchObject({
      content: "Zwei Bilder",
    });
    await expect.element(list).not.toBeInTheDocument();
  });

  it("attaches every file dropped on the conversation and sends them alone", async () => {
    const { screen, field } = await renderComposer();
    const pane = field.element().closest(".lr-message-pane")!;
    const files = carrying([
      picture("a.png"),
      picture("b.png"),
      picture("c.png"),
    ]);
    for (const type of ["dragenter", "dragover", "drop"])
      pane.dispatchEvent(
        new DragEvent(type, {
          dataTransfer: files,
          bubbles: true,
          cancelable: true,
        }),
      );
    const list = screen.getByRole("list", { name: "Anhänge" });
    await expect.element(list.getByRole("listitem")).toHaveLength(3);
    // The drop moved the keyboard to the field.
    expect(document.activeElement).toBe(field.element());

    // A file taken out again stays behind.
    await userEvent.click(
      screen.getByRole("button", { name: "b.png entfernen" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Senden" }));
    expect(sentFiles()).toEqual(["a.png", "c.png"]);
  });
});
