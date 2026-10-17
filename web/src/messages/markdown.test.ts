import { describe, expect, it } from "vitest";

import {
  parseInline,
  parseMessage,
  previewUrls,
  safeHref,
  type Inline,
} from "./markdown.ts";

const text = (text: string): Inline => ({ type: "text", text });
const link = (href: string, suppressed = false): Inline => ({
  type: "link",
  href,
  text: href,
  suppressed,
});

describe("inline markdown", () => {
  it("formats the Discord subset", () => {
    expect(parseInline("**b** *i* _i_ __u__ ~~s~~ ||x|| `c`")).toEqual([
      { type: "strong", children: [text("b")] },
      text(" "),
      { type: "em", children: [text("i")] },
      text(" "),
      { type: "em", children: [text("i")] },
      text(" "),
      { type: "underline", children: [text("u")] },
      text(" "),
      { type: "strike", children: [text("s")] },
      text(" "),
      { type: "spoiler", children: [text("x")] },
      text(" "),
      { type: "code", text: "c" },
    ]);
  });

  it("nests", () => {
    expect(parseInline("***both***")).toEqual([
      { type: "strong", children: [{ type: "em", children: [text("both")] }] },
    ]);
    expect(parseInline("*a **b** c*")).toEqual([
      {
        type: "em",
        children: [
          text("a "),
          { type: "strong", children: [text("b")] },
          text(" c"),
        ],
      },
    ]);
    expect(parseInline("||__~~x~~__||")).toEqual([
      {
        type: "spoiler",
        children: [
          {
            type: "underline",
            children: [{ type: "strike", children: [text("x")] }],
          },
        ],
      },
    ]);
  });

  it("leaves unclosed and empty markers as text", () => {
    expect(parseInline("**open")).toEqual([text("**open")]);
    expect(parseInline("a * b * c")).toEqual([text("a * b * c")]);
    expect(parseInline("****")).toEqual([text("****")]);
  });

  it("keeps underscores inside words", () => {
    expect(parseInline("snake_case_name")).toEqual([text("snake_case_name")]);
    expect(parseInline("_snake_case_ x")).toEqual([
      { type: "em", children: [text("snake_case")] },
      text(" x"),
    ]);
  });

  it("escapes markers with a backslash", () => {
    expect(parseInline("\\*not\\* \\`x\\` \\|\\|y||")).toEqual([
      text("*not* `x` ||y||"),
    ]);
    expect(parseInline("C:\\Users\\n")).toEqual([text("C:\\Users\\n")]);
  });

  it("does not format inside code", () => {
    expect(parseInline("`**x** https://a.example _y_`")).toEqual([
      { type: "code", text: "**x** https://a.example _y_" },
    ]);
    expect(parseInline("``a ` b``")).toEqual([{ type: "code", text: "a ` b" }]);
    expect(parseInline("*a `*` b*")).toEqual([
      {
        type: "em",
        children: [text("a "), { type: "code", text: "*" }, text(" b")],
      },
    ]);
  });

  it("links bare http(s) URLs without trailing punctuation", () => {
    expect(parseInline("see https://example.com/a_b_c.")).toEqual([
      text("see "),
      link("https://example.com/a_b_c"),
      text("."),
    ]);
    expect(parseInline("(https://en.wikipedia.org/wiki/Foo_(bar))")).toEqual([
      text("("),
      link("https://en.wikipedia.org/wiki/Foo_(bar)"),
      text(")"),
    ]);
    expect(parseInline("**https://example.com/**")).toEqual([
      { type: "strong", children: [link("https://example.com/")] },
    ]);
    expect(parseInline("xhttps://example.com")).toEqual([
      text("xhttps://example.com"),
    ]);
  });

  it("does not let markers inside a URL close formatting", () => {
    expect(parseInline("*go https://a.example/x*y now*")).toEqual([
      {
        type: "em",
        children: [text("go "), link("https://a.example/x*y"), text(" now")],
      },
    ]);
  });

  it("marks <url> links as suppressed and allows mailto there", () => {
    expect(parseInline("<https://example.com/>")).toEqual([
      link("https://example.com/", true),
    ]);
    expect(parseInline("<mailto:a@example.com>")).toEqual([
      {
        type: "link",
        href: "mailto:a@example.com",
        text: "mailto:a@example.com",
        suppressed: true,
      },
    ]);
  });

  it("recognises mentions outside code only", () => {
    const id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    expect(parseInline(`hi <@${id}>!`)).toEqual([
      text("hi "),
      { type: "mention", userId: id },
      text("!"),
    ]);
    expect(parseInline(`hi <@${id.toUpperCase()}>`)[1]).toEqual({
      type: "mention",
      userId: id,
    });
    expect(parseInline(`\`<@${id}>\``)).toEqual([
      { type: "code", text: `<@${id}>` },
    ]);
    expect(parseInline("<@nope>")).toEqual([text("<@nope>")]);
    expect(parseInline(`<@!${id}>`)).toEqual([text(`<@!${id}>`)]);
  });

  it("recognises @everyone and @here as whole words outside code", () => {
    expect(parseInline("hey @everyone, (@here)")).toEqual([
      text("hey "),
      { type: "everyone" },
      text(", ("),
      { type: "here" },
      text(")"),
    ]);
    expect(parseInline("**@here**")).toEqual([
      { type: "strong", children: [{ type: "here" }] },
    ]);
    for (const src of [
      "a@everyone",
      "@@here",
      "x-@here",
      "@everyones",
      "@here_",
      "@here-now",
      "`@everyone`",
      "\\@everyone",
      "mail@here.example",
    ])
      expect(
        parseInline(src).some(
          (n) => n.type === "everyone" || n.type === "here",
        ),
        src,
      ).toBe(false);
  });

  it("keeps HTML and script links as text", () => {
    for (const src of [
      "<script>alert(1)</script>",
      "<img src=x onerror=alert(1)>",
      "javascript:alert(1)",
      "<javascript:alert(1)>",
      "<data:text/html,hi>",
      "[x](javascript:alert(1))",
    ])
      expect(parseInline(src).every((n) => n.type === "text")).toBe(true);
    expect(safeHref("javascript:alert(1)")).toBeNull();
    expect(safeHref("https://")).toBeNull();
    expect(safeHref("HTTPS://Example.com")).toBe("https://example.com/");
  });

  it("stays fast on pathological input", () => {
    const started = performance.now();
    parseInline("*a _b ~~c ||d **e __f ".repeat(90));
    parseInline("**".repeat(1000));
    parseInline("*a".repeat(1000));
    parseInline("`".repeat(2000));
    expect(performance.now() - started).toBeLessThan(500);
  });
});

describe("blocks", () => {
  it("parses fenced code with and without a language", () => {
    expect(parseMessage("look:\n```ts\nconst a = **1**;\n```\ndone")).toEqual([
      { type: "paragraph", children: [text("look:")] },
      { type: "codeblock", lang: "ts", text: "const a = **1**;" },
      { type: "paragraph", children: [text("done")] },
    ]);
    expect(parseMessage("```just code```")).toEqual([
      { type: "codeblock", lang: null, text: "just code" },
    ]);
  });

  it("does not link or format inside code blocks", () => {
    expect(parseMessage("```\nhttps://a.example <@x> > q\n```")).toEqual([
      { type: "codeblock", lang: null, text: "https://a.example <@x> > q" },
    ]);
  });

  it("leaves an unclosed fence as text", () => {
    expect(parseMessage("```ts\nopen")).toEqual([
      { type: "paragraph", children: [text("```ts\nopen")] },
    ]);
  });

  it("parses quotes", () => {
    expect(parseMessage("> a\n> **b**\nc")).toEqual([
      {
        type: "quote",
        children: [text("a\n"), { type: "strong", children: [text("b")] }],
      },
      { type: "paragraph", children: [text("c")] },
    ]);
    expect(parseMessage("x\n>>> all\nof it")).toEqual([
      { type: "paragraph", children: [text("x")] },
      { type: "quote", children: [text("all\nof it")] },
    ]);
    expect(parseMessage(">not a quote")).toEqual([
      { type: "paragraph", children: [text(">not a quote")] },
    ]);
  });

  it("keeps line breaks in paragraphs", () => {
    expect(parseMessage("a\n\nb")).toEqual([
      { type: "paragraph", children: [text("a\n\nb")] },
    ]);
  });
});

describe("preview URLs", () => {
  it("takes up to three bare links outside code, spoilers and <url>", () => {
    expect(
      previewUrls(
        parseMessage(
          "https://a.example <https://b.example> ||https://c.example|| `https://d.example` https://a.example https://e.example https://f.example https://g.example\n```\nhttps://h.example\n```",
        ),
      ),
    ).toEqual([
      "https://a.example/",
      "https://e.example/",
      "https://f.example/",
    ]);
  });

  it("stays fast on many unclosed marks", () => {
    // 20k characters take ~5 ms here and took over a second while the parser
    // was quadratic, so the generous limit still catches a regression on a
    // slow runner.
    for (const unit of ["*a ", "_a "]) {
      const src = unit.repeat(Math.floor(20000 / unit.length));
      const start = performance.now();
      expect(parseInline(src)).toEqual([text(src)]);
      expect(performance.now() - start, unit).toBeLessThan(500);
    }
  });
});
