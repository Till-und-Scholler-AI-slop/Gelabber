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
const call = vi.hoisted(() => ({
  state: {
    status: "idle",
    serverId: null as string | null,
    channelId: null as string | null,
  },
}));
vi.mock("../voice/session.ts", () => ({
  useVoice: (select: (state: unknown) => unknown) => select(call.state),
  joinVoice: () => {},
  toggleSourceWatch: () => {},
}));

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

  it("in another call the button only goes to the channel", () => {
    shown.toasts = [{ ...base, live: { userId: "ada", voice: "Lounge" } }];
    call.state = { status: "joined", serverId: "srv", channelId: "sofa" };
    expect(renderToStaticMarkup(<MessageToasts />)).toMatch(
      /<button[^>]*>Zum Kanal<\/button>/,
    );
    // In that call it watches.
    call.state = { status: "joined", serverId: "srv", channelId: "lounge" };
    expect(renderToStaticMarkup(<MessageToasts />)).toMatch(
      /<button[^>]*>Zuschauen<\/button>/,
    );
    call.state = { status: "idle", serverId: null, channelId: null };
  });

  it("a message has no watch button", () => {
    shown.toasts = [{ ...base, preview: "Hallo" }];
    const html = renderToStaticMarkup(<MessageToasts />);
    expect(html).not.toContain("Zuschauen");
    expect(html).toContain("Hallo");
  });
});
