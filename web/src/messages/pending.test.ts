import { afterEach, describe, expect, it, vi } from "vitest";

import {
  addPending,
  confirmPending,
  displayPending,
  resetPendingMessages,
  usePendingMessages,
} from "./pending.ts";
import type { Message } from "./types.ts";

function row(id: string, preview?: string): Message {
  return {
    id,
    channel_id: "c",
    author: { id: "a", name: "Ada", avatar_url: null },
    content: id,
    created_at: "2026-09-28T00:00:00Z",
    edited_at: null,
    attachments: [
      {
        id: `${id}-file`,
        filename: "a.png",
        content_type: "image/png",
        size: 1,
        preview_url: preview,
      },
    ],
  };
}
const rows = () => usePendingMessages.getState().byChannel.c ?? [];
afterEach(() => {
  resetPendingMessages();
  vi.restoreAllMocks();
});

describe("pending overlay lifetime", () => {
  it("releases a confirmed send whose channel is not shown and revokes its preview", () => {
    const revoke = vi.spyOn(URL, "revokeObjectURL").mockReturnValue();
    addPending("c", row("tmp:image", "blob:image"));
    confirmPending("c", "tmp:image", row("server"));
    expect(rows()).toEqual([]);
    expect(revoke).toHaveBeenCalledWith("blob:image");
  });

  it("keeps a confirmed row while shown and releases it on leaving", () => {
    const revoke = vi.spyOn(URL, "revokeObjectURL").mockReturnValue();
    const hide = displayPending("c");
    addPending("c", row("tmp:image", "blob:image"));
    addPending("c", row("tmp:failed", "blob:failed"));
    confirmPending("c", "tmp:image", row("server"));
    expect(rows().map((m) => m.id)).toEqual(["server", "tmp:failed"]);
    expect(revoke).not.toHaveBeenCalled();
    hide();
    // The unconfirmed row still belongs to a recoverable attempt.
    expect(rows().map((m) => m.id)).toEqual(["tmp:failed"]);
    expect(revoke).toHaveBeenCalledExactlyOnceWith("blob:image");
  });
});
