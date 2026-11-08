import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { MessageToast } from "../messages/toasts.ts";

const shown = vi.hoisted(() => ({ toasts: [] as MessageToast[] }));
vi.mock("../messages/toasts.ts", () => ({
  useMessageToasts: (select: (state: unknown) => unknown) =>
    select({
      toasts: shown.toasts,
      dismiss: () => {},
      hold: () => {},
      release: () => {},
    }),
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => () => {} }));

import { MessageToasts } from "./MessageToasts.tsx";

const base: MessageToast = {
  id: 1,
  channelId: "lounge",
  serverId: "srv",
  dm: false,
  channelLabel: "Runde",
  author: "Ada",
  authorAvatar: null,
  mentioned: false,
  preview: "Ada ist live in Lounge.",
  count: 1,
  at: 0,
};

describe("message toasts", () => {
  it("a Live: the name once, the on-air tag and a way to watch", () => {
    shown.toasts = [{ ...base, live: { userId: "ada", voice: "Lounge" } }];
    const html = renderToStaticMarkup(<MessageToasts />);
    const text = html.replace(/<[^>]+>/g, "");
    expect(text.match(/Ada/g)).toHaveLength(1);
    expect(html).toContain("<b>Live</b> in Lounge");
    expect(html).toMatch(/<button[^>]*>Zuschauen<\/button>/);
    expect(html).toContain(
      'aria-label="Öffnen: Runde, Ada ist live in Lounge."',
    );
  });

  it("a message has no watch button", () => {
    shown.toasts = [{ ...base, preview: "Hallo" }];
    const html = renderToStaticMarkup(<MessageToasts />);
    expect(html).not.toContain("Zuschauen");
    expect(html).toContain("Hallo");
  });
});
