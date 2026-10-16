import { afterEach, describe, expect, it } from "vitest";

import { confirmAction, useConfirm } from "./confirm.ts";

const request = { title: "Kanal löschen?", confirmLabel: "Löschen" };

describe("confirm dialog", () => {
  afterEach(() => useConfirm.setState({ pending: null }));

  it("resolves true when confirmed and closes", async () => {
    const answer = confirmAction(request);
    const pending = useConfirm.getState().pending;
    expect(pending?.title).toBe("Kanal löschen?");
    useConfirm.getState().answer(pending!.id, true);
    await expect(answer).resolves.toBe(true);
    expect(useConfirm.getState().pending).toBeNull();
  });

  it("resolves false when cancelled", async () => {
    const answer = confirmAction(request);
    useConfirm.getState().answer(useConfirm.getState().pending!.id, false);
    await expect(answer).resolves.toBe(false);
  });

  it("cancels the open question when a new one is asked", async () => {
    const first = confirmAction(request);
    const second = confirmAction({ ...request, title: "Server verlassen?" });
    await expect(first).resolves.toBe(false);
    const pending = useConfirm.getState().pending!;
    expect(pending.title).toBe("Server verlassen?");
    useConfirm.getState().answer(pending.id, true);
    await expect(second).resolves.toBe(true);
  });

  it("ignores a late answer for a question that was replaced", async () => {
    void confirmAction(request);
    const staleId = useConfirm.getState().pending!.id;
    const second = confirmAction(request);
    useConfirm.getState().answer(staleId, true);
    expect(useConfirm.getState().pending).not.toBeNull();
    useConfirm.getState().answer(useConfirm.getState().pending!.id, false);
    await expect(second).resolves.toBe(false);
  });
});
