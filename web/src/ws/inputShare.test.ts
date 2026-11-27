import { afterEach, describe, expect, it, vi } from "vitest";

import { shareInput, type InputShare } from "./inputShare.ts";
import { PRESENCE_PULSE_MS } from "./live.ts";

const open: InputShare[] = [];
afterEach(() => {
  for (const share of open.splice(0)) share.close();
});

function tab(userId: string, now = () => 0) {
  const heard: number[] = [];
  const share = shareInput(userId, () => heard.push(now()), now);
  open.push(share);
  return { share, heard };
}

describe("input shared across tabs", () => {
  it("tells the account's other tabs, and only those", async () => {
    const a = tab("user-1");
    const b = tab("user-1");
    const stranger = tab("user-2");
    a.share.post();
    await vi.waitFor(() => expect(b.heard.length).toBe(1));
    // A tab does not hear its own input.
    expect(a.heard).toEqual([]);
    // Delivered in order: once user-2's own tab hears user-2, user-1's post
    // would have arrived before.
    const other = tab("user-2");
    stranger.share.post();
    await vi.waitFor(() => expect(other.heard.length).toBe(1));
    expect(stranger.heard).toEqual([]);
  });

  it("posts at most once per pulse period", () => {
    const posted: unknown[] = [];
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        onmessage: unknown = null;
        postMessage(message: unknown) {
          posted.push(message);
        }
        close() {}
      },
    );
    try {
      let clock = 0;
      const a = tab("user-1", () => clock);
      a.share.post();
      clock = PRESENCE_PULSE_MS - 1;
      a.share.post();
      expect(posted.length).toBe(1);
      clock = PRESENCE_PULSE_MS;
      a.share.post();
      expect(posted.length).toBe(2);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
