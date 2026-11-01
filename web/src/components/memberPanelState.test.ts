import { describe, expect, it } from "vitest";

import {
  memberPanelHidden,
  setMemberPanelVideo,
  toggleMemberPanel,
} from "./memberPanelState.ts";

describe("member list", () => {
  it("closes for pictures until the user picks, keeping the chat's choice apart", () => {
    expect(memberPanelHidden()).toBe(false);
    setMemberPanelVideo(true);
    expect(memberPanelHidden()).toBe(true);
    // Opened next to pictures: it stays open there.
    toggleMemberPanel();
    expect(memberPanelHidden()).toBe(false);
    setMemberPanelVideo(false);
    expect(memberPanelHidden()).toBe(false);
    // Closed next to a chat: the pictures keep their own choice.
    toggleMemberPanel();
    expect(memberPanelHidden()).toBe(true);
    setMemberPanelVideo(true);
    expect(memberPanelHidden()).toBe(false);
    setMemberPanelVideo(false);
    expect(memberPanelHidden()).toBe(true);
  });
});
