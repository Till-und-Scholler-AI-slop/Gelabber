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
import { setReplyTarget } from "../messages/replyTarget.ts";
import type { Message } from "../messages/types.ts";
import { Composer } from "./Composer.tsx";
import { forwardTypingToComposer } from "./composerFocus.ts";
// The field's height comes from the stylesheet, with the app's box model
// (Tailwind's preflight).
import "./chat.css";

const boxes = document.createElement("style");
boxes.textContent = "*, ::before, ::after { box-sizing: border-box; }";
document.head.append(boxes);

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

/** A conversation page as the workspace shows it: the sidebar's link, the
 * header's search, the pane `height` tall with the composer at its end. */
function Page({
  channelId,
  onSent = () => {},
  height,
}: {
  channelId: string;
  onSent?: () => void;
  height?: number;
}) {
  return (
    <QueryClientProvider client={client}>
      <nav>
        <a href="#elsewhere">#anderswo</a>
      </nav>
      <input type="search" aria-label="Suche" />
      <button type="button">Glocke</button>
      <div
        className="lr-message-pane"
        style={
          height === undefined
            ? undefined
            : { display: "flex", flexDirection: "column", height }
        }
      >
        {height === undefined ? null : <div style={{ flex: 1 }} />}
        <Composer
          handle={null}
          channelId={channelId}
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
    </QueryClientProvider>
  );
}
let client = new QueryClient();

async function renderComposer(height?: number) {
  const onSent = vi.fn();
  client = new QueryClient();
  const screen = await render(
    <Page channelId={`c${channel}`} onSent={onSent} height={height} />,
  );
  const field = screen.getByRole("combobox", {
    name: "Nachricht an #allgemein",
  });
  // Past the frame in which opening it may move the keyboard (GEL-143).
  await frames();
  return { screen, field, onSent };
}

/** The frame the composer waits for, and one more to be sure it is over. */
async function frames() {
  for (let i = 0; i < 2; i += 1)
    await new Promise((done) => requestAnimationFrame(done));
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

  it("breaks the line on Enter inside a code block that is still open", async () => {
    const { field } = await renderComposer();
    await userEvent.click(field);
    await userEvent.keyboard("```js{Enter}let a = 1;{Enter}");
    expect(sent.mutate).not.toHaveBeenCalled();
    await expect.element(field).toHaveValue("```js\nlet a = 1;\n");
    // Closed, the block is sent with the next Enter.
    await userEvent.keyboard("```{Enter}");
    expect(sent.mutate).toHaveBeenCalledOnce();
    expect(sent.mutate.mock.calls[0]![0]).toMatchObject({
      content: "```js\nlet a = 1;\n```",
    });
  });

  it("does not send on the Enter that ends an input method's composition", async () => {
    const { field } = await renderComposer();
    await userEvent.click(field);
    await userEvent.keyboard("Kanji");
    field.element().dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        isComposing: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(sent.mutate).not.toHaveBeenCalled();
    // The next, plain Enter does send: the handler was listening.
    await userEvent.keyboard("{Enter}");
    expect(sent.mutate).toHaveBeenCalledOnce();
  });

  it("sends a reply to the message answered and closes its banner", async () => {
    const answered: Message = {
      id: "m1",
      channel_id: `c${channel}`,
      author: { id: "u2", name: "Bea", avatar_url: null },
      content: "Wer kommt mit?",
      created_at: "2026-11-27T18:00:00Z",
      edited_at: null,
      attachments: [],
      reactions: [],
      mentions: [],
      mention_everyone: false,
    };
    setReplyTarget(`c${channel}`, answered);
    const { screen, field } = await renderComposer();
    const banner = screen.getByText(/^Antwort an/);
    await expect.element(banner).toBeVisible();
    expect(banner.element().textContent).toMatch(/^Antwort an Bea\b/);
    await userEvent.click(field);
    await userEvent.keyboard("Ich{Enter}");
    expect(sent.mutate.mock.calls[0]![0]).toMatchObject({
      content: "Ich",
      replyTo: answered,
    });
    await expect.element(banner).not.toBeInTheDocument();
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
    (document.activeElement as HTMLElement | null)?.blur();
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

describe("composer focus (GEL-143)", () => {
  /** Opens the next conversation in the same page, as the router does. */
  async function open(screen: Awaited<ReturnType<typeof render>>) {
    channel += 1;
    await screen.rerender(<Page channelId={`c${channel}`} />);
    await frames();
  }

  it("takes the keyboard when the page opens on a conversation", async () => {
    const { field } = await renderComposer();
    expect(document.activeElement).toBe(field.element());
  });

  it("takes the keyboard when a conversation opens from the sidebar", async () => {
    const { screen, field } = await renderComposer();
    const link = screen.getByRole("link", { name: "#anderswo" });
    (link.element() as HTMLElement).focus();
    await open(screen);
    expect(document.activeElement).toBe(field.element());
  });

  it("leaves the keyboard in the search or another field", async () => {
    const { screen } = await renderComposer();
    const search = screen.getByRole("searchbox", { name: "Suche" });
    (search.element() as HTMLElement).focus();
    await open(screen);
    expect(document.activeElement).toBe(search.element());
  });

  it("does not open the on-screen keyboard of a touch screen", async () => {
    const { screen } = await renderComposer();
    const link = screen.getByRole("link", { name: "#anderswo" });
    (link.element() as HTMLElement).focus();
    const real = window.matchMedia.bind(window);
    const touch = vi
      .spyOn(window, "matchMedia")
      .mockImplementation((query) =>
        query === "(pointer: coarse)"
          ? ({ ...real(query), matches: true } as MediaQueryList)
          : real(query),
      );
    try {
      await open(screen);
    } finally {
      touch.mockRestore();
    }
    expect(document.activeElement).toBe(link.element());
  });

  it("gets what is typed while nothing else has the keyboard", async () => {
    const stop = forwardTypingToComposer();
    try {
      const { screen, field } = await renderComposer();
      (document.activeElement as HTMLElement | null)?.blur();
      expect(document.activeElement).toBe(document.body);
      await userEvent.keyboard("Hal");
      await expect.element(field).toHaveValue("Hal");
      expect(document.activeElement).toBe(field.element());

      // From the sidebar's link too, with Alt Gr's characters.
      const link = screen.getByRole("link", { name: "#anderswo" });
      (link.element() as HTMLElement).focus();
      await userEvent.keyboard("lo @");
      await expect.element(field).toHaveValue("Hallo @");

      // A button keeps its keys, and so does a key something else used.
      const bell = screen.getByRole("button", { name: "Glocke" });
      (bell.element() as HTMLElement).focus();
      await userEvent.keyboard("x");
      expect(document.activeElement).toBe(bell.element());
      (bell.element() as HTMLElement).blur();
      const taken = (event: KeyboardEvent) => event.preventDefault();
      window.addEventListener("keydown", taken);
      try {
        await userEvent.keyboard("k");
      } finally {
        window.removeEventListener("keydown", taken);
      }
      expect(document.activeElement).toBe(document.body);
      await expect.element(field).toHaveValue("Hallo @");
    } finally {
      stop();
    }
  });
});

describe("composer height (GEL-143)", () => {
  const height = (field: Element) => field.getBoundingClientRect().height;

  it("grows with the draft up to half the conversation, then scrolls", async () => {
    const { field } = await renderComposer(800);
    await userEvent.click(field);
    const element = field.element() as HTMLTextAreaElement;
    expect(height(element)).toBe(34);
    await userEvent.keyboard(
      "eins{Shift>}{Enter}{/Shift}zwei{Shift>}{Enter}{/Shift}drei",
    );
    // Two more lines of 22px.
    expect(height(element)).toBe(78);

    await userEvent.keyboard(
      "{Shift>}{Enter}{/Shift}noch eine Zeile".repeat(30),
    );
    // Half of the 800px pane, not the phone's 160px.
    expect(height(element)).toBe(400);
    expect(element.scrollHeight).toBeGreaterThan(element.clientHeight);
  });

  it("is at least 160px in a short conversation", async () => {
    const { field } = await renderComposer(240);
    await userEvent.click(field);
    const element = field.element() as HTMLTextAreaElement;
    await userEvent.keyboard("{Shift>}{Enter}{/Shift}Zeile".repeat(20));
    expect(height(element)).toBe(160);
  });
});
