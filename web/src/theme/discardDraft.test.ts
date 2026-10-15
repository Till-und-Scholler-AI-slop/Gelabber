import { afterEach, describe, expect, it } from "vitest";

import { useConfirm } from "../components/confirm.ts";
import { blockThemeLeave } from "./discardDraft.ts";

// The decision behind `useBlocker` in ThemesPanel: the router itself needs a
// DOM, so this drives the same function it calls and answers the dialog.
describe("leaving the theme editor", () => {
  afterEach(() => useConfirm.setState({ pending: null }));

  it("lets a clean editor go without asking", async () => {
    await expect(blockThemeLeave(false)).resolves.toBe(false);
    expect(useConfirm.getState().pending).toBeNull();
  });

  it("asks for a dirty draft and stays on Abbrechen", async () => {
    const block = blockThemeLeave(true);
    const pending = useConfirm.getState().pending;
    expect(pending?.title).toBe("Ungespeicherten Theme-Entwurf verwerfen?");
    expect(pending?.confirmLabel).toBe("Verwerfen");
    // Destructive, so the dialog opens on Abbrechen.
    expect(pending?.tone).not.toBe("neutral");
    useConfirm.getState().answer(pending!.id, false);
    await expect(block).resolves.toBe(true);
  });

  it("navigates once the draft is discarded", async () => {
    const block = blockThemeLeave(true);
    useConfirm.getState().answer(useConfirm.getState().pending!.id, true);
    await expect(block).resolves.toBe(false);
  });

  it("stays when another question replaces the open one", async () => {
    const block = blockThemeLeave(true);
    void blockThemeLeave(true);
    await expect(block).resolves.toBe(true);
  });
});
