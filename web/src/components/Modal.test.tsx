import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useConfirm } from "./confirm.ts";
import { Modal } from "./Modal.tsx";
import {
  closesOnBackdrop,
  DISCARD_CHANGES,
  focusLeftWith,
  requestClose,
  restoreFocus,
} from "./modalClose.ts";

// Markup plus the close decisions: node has no <dialog>, so showModal and
// the events themselves come from the browser.
describe("modal markup", () => {
  const html = renderToStaticMarkup(
    <Modal open onClose={() => {}} title="Server erstellen" footer={<b>OK</b>}>
      <input aria-label="Name" />
    </Modal>,
  );

  it("names the dialog by its title, which sits outside the scrolling body", () => {
    const titleId = /aria-labelledby="([^"]+)"/.exec(html)?.[1];
    expect(html).toMatch(
      new RegExp(
        `<header class="gel-modal__header"><h2 id="${titleId}">Server erstellen</h2></header><div class="gel-modal__body">`,
      ),
    );
  });

  it("always has a visible way out, after the content in tab order", () => {
    expect(html).toContain('aria-label="Schließen"');
    expect(html.indexOf('aria-label="Name"')).toBeLessThan(
      html.indexOf('aria-label="Schließen"'),
    );
  });

  it("pins the footer under the body", () => {
    expect(html).toContain(
      '<footer class="gel-modal__footer"><b>OK</b></footer>',
    );
  });

  it("renders no content while closed", () => {
    const closed = renderToStaticMarkup(
      <Modal open={false} onClose={() => {}} title="Zu">
        <p>Inhalt</p>
      </Modal>,
    );
    expect(closed).not.toContain("Inhalt");
    expect(closed).not.toContain("Schließen");
  });
});

describe("closing a modal", () => {
  const box = { top: 100, right: 500, bottom: 400, left: 100 };

  afterEach(() => useConfirm.setState({ pending: null }));

  it("closes on a backdrop click, but not on a click on its own padding", () => {
    expect(closesOnBackdrop({ clientX: 50, clientY: 200 }, box, false)).toBe(
      true,
    );
    expect(closesOnBackdrop({ clientX: 120, clientY: 110 }, box, false)).toBe(
      false,
    );
  });

  it("ignores a backdrop click while there is unsaved input", () => {
    expect(closesOnBackdrop({ clientX: 50, clientY: 200 }, box, true)).toBe(
      false,
    );
  });

  it("closes at once on Esc without unsaved input", async () => {
    const onClose = vi.fn();
    const confirm = vi.fn(async () => true);
    expect(await requestClose({ dirty: false, onClose, confirm })).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("asks before discarding unsaved input, and stays open on Abbrechen", async () => {
    const onClose = vi.fn();
    const closing = requestClose({ dirty: true, onClose });
    const pending = useConfirm.getState().pending;
    expect(pending).toMatchObject({ title: "Änderungen verwerfen?" });
    useConfirm.getState().answer(pending!.id, false);
    expect(await closing).toBe(false);
    expect(onClose).not.toHaveBeenCalled();

    const discarding = requestClose({ dirty: true, onClose });
    useConfirm.getState().answer(useConfirm.getState().pending!.id, true);
    expect(await discarding).toBe(true);
    expect(onClose).toHaveBeenCalledOnce();
    expect(DISCARD_CHANGES.confirmLabel).toBe("Verwerfen");
  });

  it("returns focus to the opener while it is still on screen", () => {
    const opener = {
      isConnected: true,
      getClientRects: () => ({ length: 1 }),
      focus: vi.fn(),
    };
    expect(restoreFocus(opener)).toBe(true);
    expect(opener.focus).toHaveBeenCalledWith({ preventScroll: true });

    const gone = { ...opener, isConnected: false, focus: vi.fn() };
    const hidden = {
      ...opener,
      getClientRects: () => ({ length: 0 }),
      focus: vi.fn(),
    };
    expect(restoreFocus(gone)).toBe(false);
    expect(restoreFocus(hidden)).toBe(false);
    expect(restoreFocus(null)).toBe(false);
    expect(gone.focus).not.toHaveBeenCalled();
    expect(hidden.focus).not.toHaveBeenCalled();
  });
});

describe("focus when a surface closes", () => {
  const element = (name = "BUTTON") =>
    ({ nodeName: name }) as unknown as HTMLElement;
  const surface = (...inside: Element[]) => ({
    contains: (node: Node | null) => inside.includes(node as Element),
  });

  it("is the closing surface's when it is inside it or nowhere", () => {
    const button = element();
    expect(focusLeftWith(surface(button), button)).toBe(true);
    expect(focusLeftWith(surface(), null)).toBe(true);
    expect(focusLeftWith(surface(), element("BODY"))).toBe(true);
    expect(focusLeftWith(surface(), element())).toBe(false);
  });
});
