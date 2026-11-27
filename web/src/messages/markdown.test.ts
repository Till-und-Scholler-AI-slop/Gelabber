import { describe, expect, it } from "vitest";

import {
  parseInline,
  parseMessage,
  plainText,
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

// The parser's work on a hostile input, counted instead of timed so the
// bound holds on any machine: each call of a string method below, plus the
// characters it scans or copies, and each regular-expression `exec` or
// `test`. They are wrapped only while the parse runs, which is
// synchronous, so nothing else is counted.
//
// Invisible to the count: a plain `src[k]` loop that makes none of these
// calls, backtracking inside one regular expression, and `new URL` in
// `safeHref`. `slice` charges every character it copies, so `src.slice(i)`
// at each `<` (for the mention and angle-link patterns) counts the rest of
// the string there; an input with many `<` would look quadratic to this
// count although it is not.
type StringMethod = (this: string, ...args: unknown[]) => unknown;
const scanned: Record<
  string,
  (self: string, args: unknown[], result: unknown) => number
> = {
  startsWith: () => 1,
  includes: () => 1,
  indexOf: (self, args, result) =>
    (result === -1 ? self.length : Number(result) + 1) - Number(args[1] ?? 0),
  slice: (_self, _args, result) => String(result).length,
  split: (self) => self.length,
  substr: (_self, _args, result) => String(result).length,
  substring: (_self, _args, result) => String(result).length,
  lastIndexOf: (self) => self.length,
  charAt: () => 1,
  charCodeAt: () => 1,
  codePointAt: () => 1,
  at: () => 1,
  endsWith: () => 1,
  match: (self) => self.length,
  replace: (self) => self.length,
  search: (self) => self.length,
};
function parseWork(src: string): number {
  const proto = String.prototype as unknown as Record<string, StringMethod>;
  const saved = Object.keys(scanned).map(
    (name) => [name, proto[name]] as const,
  );
  let work = 0;
  for (const [name, original] of saved)
    proto[name] = function (...args) {
      const result = original.apply(this, args);
      work += 1 + scanned[name](this, args, result);
      return result;
    };
  const re = RegExp.prototype as unknown as Record<
    string,
    (this: RegExp, s: string) => unknown
  >;
  const reSaved = (["exec", "test"] as const).map(
    (name) => [name, re[name]] as const,
  );
  for (const [name, original] of reSaved)
    re[name] = function (input) {
      work += 1;
      return original.call(this, input);
    };
  try {
    parseInline(src);
  } finally {
    for (const [name, original] of saved) proto[name] = original;
    for (const [name, original] of reSaved) re[name] = original;
  }
  return work;
}

/** Doubling the input at most doubles the work, with room for a little
 * more: a parser that rescans (the slow paths these tests guard) about
 * triples to quadruples it. */
function expectLinearWork(make: (n: number) => string, n: number) {
  const once = parseWork(make(n));
  expect(parseWork(make(2 * n)), make(4).slice(0, 12)).toBeLessThanOrEqual(
    2.5 * once + 1_000,
  );
}

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
      "mastodon.social/@here",
      "https://mastodon.social/@here",
      "https://example.org/?to=@everyone",
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

  it("stays linear on pathological input", () => {
    expectLinearWork((n) => "*a _b ~~c ||d **e __f ".repeat(n), 45);
    expectLinearWork((n) => "**".repeat(n), 500);
    expectLinearWork((n) => "*a".repeat(n), 500);
    expectLinearWork((n) => "`".repeat(n), 1000);
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

  it("stays linear on many unclosed marks", () => {
    // The parser was quadratic here once: it searched for a closer again
    // after every opener of a mark that had none.
    for (const unit of ["*a ", "_a "]) {
      const src = unit.repeat(2000);
      expect(parseInline(src)).toEqual([text(src)]);
      expectLinearWork((n) => unit.repeat(n), 1000);
    }
  });

  it("stays linear on a bare link with many trailing brackets", () => {
    // Trimming used to rescan the URL for every trailing bracket, and every
    // unclosed mark around it repeated that.
    expectLinearWork(
      (n) => "||**__~~*_http://a.b/" + ")".repeat(n) + " _*~~__**||",
      980,
    );
    expect(previewUrls(parseMessage("(http://a.b/(c)))]"))).toEqual([
      "http://a.b/(c)",
    ]);
  });
});

describe("plain text", () => {
  it("drops the marks and keeps what they mark", () => {
    expect(plainText("Ich bin dabei. **Film oder Spiel?**")).toBe(
      "Ich bin dabei. Film oder Spiel?",
    );
    expect(plainText("*a* __b__ ~~c~~ `d` \\*e\\*")).toBe("a b c d *e*");
    expect(plainText("> zitiert\nund ```rs\nfn x() {}\n``` danach")).toBe(
      "zitiert und fn x() {} danach",
    );
  });

  it("hides spoilers and keeps mentions and links as text", () => {
    expect(plainText("sag ||Snape stirbt|| nicht")).toBe("sag ▒▒▒ nicht");
    const id = "0b9e4a9c-58e2-4f4e-9d65-9b3b0b8c7d11";
    expect(plainText(`hi <@${id}> @here <https://a.b/>`)).toBe(
      `hi <@${id}> @here https://a.b/`,
    );
  });

  it("drops the marks a cut excerpt leaves unclosed", () => {
    expect(plainText("so **halb…")).toBe("so halb…");
    expect(plainText("so **halb fertig*…")).toBe("so halb fertig…");
    expect(plainText("~~weg__ und _da…")).toBe("weg__ und da…");
    // Only a cut excerpt: a whole message keeps what it typed.
    expect(plainText("so **halb")).toBe("so **halb");
    expect(plainText("a * b…")).toBe("a * b…");
  });
});
