import { describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import { hidesContent, notificationText } from "./notificationPrivacy.ts";

const shown = {
  title: "Ada hat dich erwähnt · #allgemein",
  body: "@Bob das Passwort ist hunter2",
  author: "Ada",
};

describe("notificationText", () => {
  it("shows the message unless it is hidden", () => {
    expect(notificationText(false, shown)).toEqual({
      title: shown.title,
      body: shown.body,
    });
  });

  it("says only who wrote when the content is hidden", () => {
    expect(notificationText(true, shown)).toEqual({
      title: "Neue Nachricht von Ada",
      body: "",
    });
  });
});

describe("hidesContent", () => {
  it("follows the account's switch once it is known", () => {
    expect(hidesContent({ data: { hide_content: true }, error: null })).toBe(
      true,
    );
    expect(hidesContent({ data: { hide_content: false }, error: null })).toBe(
      false,
    );
  });

  it("hides while the switch is not known yet", () => {
    expect(hidesContent(undefined)).toBe(true);
    expect(hidesContent({ data: undefined, error: null })).toBe(true);
  });

  it("hides when the last fetch failed, even with an older answer", () => {
    const offline = new ApiError("network", 0, "offline");
    expect(hidesContent({ data: undefined, error: offline })).toBe(true);
    expect(
      hidesContent({ data: { hide_content: false }, error: offline }),
    ).toBe(true);
    const busy = new ApiError("internal", 500, "busy");
    expect(hidesContent({ data: undefined, error: busy })).toBe(true);
  });

  it("shows the message on a server from before the switch", () => {
    const missing = new ApiError("not_found", 404, "not found");
    expect(hidesContent({ data: undefined, error: missing })).toBe(false);
  });
});
