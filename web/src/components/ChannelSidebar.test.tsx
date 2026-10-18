import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { resetUserVolumesForTests } from "../voice/userVolume.ts";
import { ParticipantRowView } from "./ChannelSidebar.tsx";

afterEach(() => resetUserVolumesForTests());

const row = {
  kind: "participant" as const,
  key: "p-u-bob",
  member: { user_id: "u-bob", name: "Bob", avatar_url: null },
  flags: { channelId: "voice", muted: true, deafened: false },
  live: false,
};

// Markup only: node has no DOM to click. The row is a button so a tap,
// the keyboard and iOS (no contextmenu on long-press) all open the profile
// with its volume controls.
describe("sidebar voice participant", () => {
  it("is a button that names what it opens and keeps the voice state", () => {
    const html = renderToStaticMarkup(
      <ParticipantRowView row={row} serverId="srv" onOpenProfile={() => {}} />,
    );
    expect(html).toMatch(
      /^<button type="button" class="sidebar-participant" title="Profil und Lautstärke">/,
    );
    expect(html).toContain("Bob");
    expect(html).toContain('<span class="sr-only">stumm</span>');
    expect(html).not.toContain("Lokal stummgeschaltet");
  });
});
