import { describe, expect, it } from "vitest";

import {
  fromWire,
  insertMention,
  mentionAt,
  mentionSuggestions,
  mentionsMe,
  namesIn,
  splitMentions,
  toWire,
} from "./mentions.ts";

const ADA = "0190f7a2-0000-7000-8000-00000000000a";
const BOB = "0190f7a2-0000-7000-8000-00000000000b";
const LOVELACE = "0190f7a2-0000-7000-8000-00000000000c";
const members = [
  { id: ADA, name: "Ada" },
  { id: BOB, name: "Bob" },
  { id: LOVELACE, name: "Ada Lovelace" },
];

describe("mentions", () => {
  it("knows whom the server said a message mentions", () => {
    expect(mentionsMe({ mentions: [BOB] }, BOB)).toBe(true);
    expect(mentionsMe({ mentions: [BOB] }, ADA)).toBe(false);
    expect(mentionsMe({ mentions: [], mention_everyone: true }, ADA)).toBe(
      true,
    );
    // Older API snapshots carry neither.
    expect(mentionsMe({}, ADA)).toBe(false);
    expect(mentionsMe({ mention_everyone: true }, undefined)).toBe(false);
  });

  it("splits text into runs and mentions, leaving code alone", () => {
    expect(
      splitMentions(
        `Hi <@${BOB}>, @everyone ` +
          "`<@" +
          ADA +
          ">`" +
          ` <@${"0190f7a2-0000-7000-8000-0000000000ff"}>`,
        members,
      ),
    ).toEqual([
      { kind: "text", text: "Hi " },
      { kind: "user", id: BOB, name: "Bob" },
      { kind: "text", text: ", " },
      { kind: "everyone" },
      { kind: "text", text: " `<@" + ADA + ">` " },
      {
        kind: "user",
        id: "0190f7a2-0000-7000-8000-0000000000ff",
        name: null,
      },
    ]);
    expect(splitMentions("mail a@here.org", members)).toEqual([
      { kind: "text", text: "mail a@here.org" },
    ]);
    expect(splitMentions("(@here)", members)).toEqual([
      { kind: "text", text: "(" },
      { kind: "here" },
      { kind: "text", text: ")" },
    ]);
  });

  it("leaves escaped mentions as text, as the server does", () => {
    expect(splitMentions(`\\<@${BOB}> \\@here`, members)).toEqual([
      { kind: "text", text: `\\<@${BOB}> \\@here` },
    ]);
    expect(splitMentions(`\\\\<@${BOB}>`, members)).toEqual([
      { kind: "text", text: "\\\\" },
      { kind: "user", id: BOB, name: "Bob" },
    ]);
  });

  it("swaps names and tokens both ways", () => {
    const wire = toWire("@Ada Lovelace und @Ada, nicht @Adam `@Bob`", members);
    expect(wire).toBe(`<@${LOVELACE}> und <@${ADA}>, nicht @Adam \`@Bob\``);
    expect(fromWire(wire, members)).toBe(
      "@Ada Lovelace und @Ada, nicht @Adam `@Bob`",
    );
    // Unknown ids stay tokens; plain text is untouched.
    const stranger = "<@0190f7a2-0000-7000-8000-0000000000ff>";
    expect(fromWire(stranger, members)).toBe(stranger);
    expect(toWire("kein at", members)).toBe("kein at");
    expect(toWire("a@Bob.de", members)).toBe("a@Bob.de");
    expect(toWire("@Bob!", members)).toBe(`<@${BOB}>!`);
  });

  it("leaves a shared name as text unless one was picked", () => {
    const OTHER = "0190f7a2-0000-7000-8000-00000000000d";
    const twins = [...members, { id: OTHER, name: "Bob" }];
    expect(toWire("@Bob hi", twins)).toBe("@Bob hi");
    expect(toWire("@Bob hi", twins, new Map([["Bob", OTHER]]))).toBe(
      `<@${OTHER}> hi`,
    );
    // An edit keeps whom the message named before.
    const sent = namesIn(`<@${OTHER}> hi`, twins);
    expect(toWire("@Bob hallo", twins, sent)).toBe(`<@${OTHER}> hallo`);
    // A member called "everyone" does not swallow the keyword.
    const crowd = [...members, { id: OTHER, name: "everyone" }];
    expect(toWire("@everyone", crowd)).toBe("@everyone");
  });

  it("finds the mention being typed and completes it", () => {
    expect(mentionAt("hi @Bo", 6)).toEqual({ start: 3, query: "Bo" });
    expect(mentionAt("@", 1)).toEqual({ start: 0, query: "" });
    expect(mentionAt("mail a@b", 8)).toBeNull();
    expect(mentionAt("hi @Bo there", 12)).toBeNull();
    const at = mentionAt("hi @Bo und", 6)!;
    expect(
      insertMention("hi @Bo und", at, { kind: "user", id: BOB, name: "Bob" }),
    ).toEqual({
      text: "hi @Bob und",
      caret: 8,
    });
    expect(
      insertMention("@ev", { start: 0, query: "ev" }, { kind: "everyone" }),
    ).toEqual({
      text: "@everyone ",
      caret: 10,
    });
  });

  it("suggests members by prefix, then by part of the name", () => {
    const names = (q: string, keywords = true) =>
      mentionSuggestions(q, members, { me: BOB, keywords }).map((s) =>
        s.kind === "user" ? s.name : `@${s.kind}`,
      );
    expect(names("ada")).toEqual(["Ada", "Ada Lovelace"]);
    expect(names("love")).toEqual(["Ada Lovelace"]);
    // Not oneself.
    expect(names("bo")).toEqual([]);
    expect(names("e")).toEqual(["Ada Lovelace", "@everyone"]);
    expect(names("h")).toEqual(["@here"]);
    expect(names("h", false)).toEqual([]);
  });
});
