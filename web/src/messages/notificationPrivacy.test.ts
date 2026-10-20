import { describe, expect, it } from "vitest";

import { notificationText } from "./notificationPrivacy.ts";

const shown = {
  title: "Ada hat dich erwähnt · #allgemein",
  body: "@Bob das Passwort ist hunter2",
  author: "Ada",
};

describe("notificationText", () => {
  it("shows the message unless the account hides it", () => {
    expect(notificationText({ hide_content: false }, shown)).toEqual({
      title: shown.title,
      body: shown.body,
    });
  });

  it("says only who wrote when the content is hidden", () => {
    expect(notificationText({ hide_content: true }, shown)).toEqual({
      title: "Neue Nachricht von Ada",
      body: "",
    });
  });

  it("follows the server's default while the setting is unknown", () => {
    expect(notificationText(undefined, shown).body).toBe(shown.body);
  });
});
