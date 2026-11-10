// The public page templates (render.tsx): static, scriptless, in both
// languages, with every optional part inside its block.
import { describe, expect, it } from "vitest";

import { public as publicPages } from "../i18n/catalogs/public.ts";
import { KEY } from "./keys.ts";
import { markerLines, renderPublicTemplates } from "./render.tsx";
import { SELF_HOST_COMMANDS } from "./text.ts";

const STYLESHEET = "/assets/public-test.css";
const templates = renderPublicTemplates({ stylesheet: STYLESHEET });
const byFile = new Map(templates.map((t) => [t.file, t.html]));
const page = (file: string) => byFile.get(file) ?? "";

const PUBLIC_URLS = [
  "/de/",
  "/en/",
  "/de/impressum",
  "/de/datenschutz",
  "/de/nutzungsbedingungen",
  "/en/imprint",
  "/en/privacy",
  "/en/terms",
];

const LEGAL_BLOCKS: Record<string, string> = {
  "/de/impressum": KEY.imprint,
  "/en/imprint": KEY.imprint,
  "/de/datenschutz": KEY.privacy,
  "/en/privacy": KEY.privacy,
  "/de/nutzungsbedingungen": KEY.terms,
  "/en/terms": KEY.terms,
};

const decode = (html: string) =>
  html
    .replaceAll("&#x27;", "'")
    .replaceAll("&#39;", "'")
    .replaceAll("&quot;", '"')
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&");

const anchors = (html: string) =>
  [...html.matchAll(/<a\b[^>]*\bhref="([^"]*)"/g)].map((m) => m[1]);

/** The keys of the blocks that enclose each line of a template. */
function blockLines(html: string): { line: string; keys: string[] }[] {
  const open: string[] = [];
  const lines: { line: string; keys: string[] }[] = [];
  for (const line of html.split("\n")) {
    const start = line.match(/^<!--if:([A-Z_]+)-->$/);
    const end = line.match(/^<!--end:([A-Z_]+)-->$/);
    if (start) open.push(start[1]);
    else if (end) expect(open.pop()).toBe(end[1]);
    else lines.push({ line, keys: [...open] });
  }
  expect(open).toEqual([]);
  return lines;
}

describe("public page templates", () => {
  it("are the landing page and the three legal pages in both languages", () => {
    expect(templates.map((t) => t.file).sort()).toEqual([
      "de/datenschutz.html",
      "de/impressum.html",
      "de/index.html",
      "de/nutzungsbedingungen.html",
      "en/imprint.html",
      "en/index.html",
      "en/privacy.html",
      "en/terms.html",
    ]);
  });

  it.each(templates)(
    "$file has no script and states its language",
    ({ file, html }) => {
      expect(html).not.toMatch(/<script/i);
      expect(html).not.toMatch(/\son[a-z]+=/i);
      expect(html.startsWith("<!doctype html>\n")).toBe(true);
      expect(html).toContain(`<html lang="${file.slice(0, 2)}">`);
      expect(html).toContain(`<link rel="stylesheet" href="${STYLESHEET}">`);
      expect(html).not.toMatch(/gelabber-(if|fragment)/);
    },
  );

  it.each(templates)("$file links only to known places", ({ html }) => {
    for (const href of anchors(html)) {
      const known =
        PUBLIC_URLS.includes(href) ||
        href === "/" ||
        href === "/login" ||
        href === "#self-host" ||
        /^(mailto:)?\{\{[A-Z_]+\}\}(\?subject=[^"]+)?$/.test(href);
      expect(known, href).toBe(true);
    }
  });

  it.each(templates)(
    "$file keeps every optional part in its block",
    ({ html }) => {
      const optional: Record<string, string> = {
        [KEY.publicUrl]: KEY.publicUrl,
        [KEY.operatorPhone]: KEY.operatorPhone,
        [KEY.contactEmail]: KEY.contactEmail,
        [KEY.inviteUrl]: KEY.inviteUrl,
        [KEY.inviteEmail]: KEY.inviteEmail,
      };
      for (const { line, keys } of blockLines(html)) {
        for (const [placeholder, block] of Object.entries(optional))
          if (line.includes(`{{${placeholder}}}`))
            expect(keys, `${placeholder} in ${line}`).toContain(block);
        // The language switch of a legal page links to the page itself.
        if (line.includes('hrefLang="')) continue;
        for (const href of anchors(line))
          if (LEGAL_BLOCKS[href])
            expect(keys, href).toContain(LEGAL_BLOCKS[href]);
      }
    },
  );

  it.each(["de", "en"] as const)(
    "the %s landing page has the catalog's text and the commands",
    (locale) => {
      const html = decode(page(`${locale}/index.html`));
      for (const [key, text] of Object.entries(publicPages[locale])) {
        if (!key.startsWith("landing.") || typeof text !== "string") continue;
        if (key === "landing.invite.subject") continue;
        expect(html, key).toContain(
          text.replace("{email}", `{{${KEY.contactEmail}}}`),
        );
      }
      expect(html).toContain(`<code>${SELF_HOST_COMMANDS.join("\n")}</code>`);
      expect(html).toContain(
        `href="mailto:{{${KEY.inviteEmail}}}?subject=${encodeURIComponent(publicPages[locale]["landing.invite.subject"])}"`,
      );
      expect(html).toContain(
        `src="/images/landing/app-${locale}.png" width="1280" height="800"`,
      );
      expect(html).toContain('id="self-host"');
    },
  );

  it("marks the current language in the landing page's footer", () => {
    expect(page("de/index.html")).toMatch(
      /<a href="\/de\/"[^>]*aria-current="page"/,
    );
    expect(page("en/index.html")).toMatch(
      /<a href="\/en\/"[^>]*aria-current="page"/,
    );
    expect(page("en/index.html")).not.toMatch(
      /<a href="\/de\/"[^>]*aria-current/,
    );
  });

  it("links the languages for search engines, inside APP_PUBLIC_URL", () => {
    const origin = `{{${KEY.publicUrl}}}`;
    for (const locale of ["de", "en"] as const) {
      const html = page(`${locale}/index.html`);
      expect(html).toContain(
        `<link rel="canonical" href="${origin}/${locale}/">`,
      );
      expect(html).toContain(
        `<link rel="alternate" hreflang="de" href="${origin}/de/">`,
      );
      expect(html).toContain(
        `<link rel="alternate" hreflang="en" href="${origin}/en/">`,
      );
      expect(html).toContain(
        `<link rel="alternate" hreflang="x-default" href="${origin}/">`,
      );
    }
    const imprint = page("de/impressum.html");
    expect(imprint).toContain(
      `<link rel="alternate" hreflang="en" href="${origin}/en/imprint">`,
    );
    expect(imprint).toContain(
      `<link rel="canonical" href="${origin}/de/impressum">`,
    );
  });

  it("gives each legal page its own fragment slot and the other language's link", () => {
    for (const [file, name, other] of [
      ["de/impressum.html", "imprint", "/en/imprint"],
      ["de/datenschutz.html", "privacy", "/en/privacy"],
      ["de/nutzungsbedingungen.html", "terms", "/en/terms"],
      ["en/imprint.html", "imprint", "/de/impressum"],
      ["en/privacy.html", "privacy", "/de/datenschutz"],
      ["en/terms.html", "terms", "/de/nutzungsbedingungen"],
    ]) {
      const html = page(file);
      expect(html, file).toContain(`\n<!--gelabber:fragment ${name}-->\n`);
      expect(html, file).toContain(`lang="{{${KEY.fragmentLang}}}"`);
      expect(html, file).toContain(`<a href="${other}"`);
      expect(html, file).toMatch(/aria-current="page">[^<]+<\/a><\/li>/);
    }
  });

  it("writes the Impressum from the operator's values, without the design note", () => {
    const html = page("de/impressum.html");
    for (const key of [
      KEY.operatorName,
      KEY.operatorStreet,
      KEY.operatorCity,
      KEY.operatorEmail,
      KEY.contentResponsible,
      KEY.reportEmail,
    ])
      expect(html).toContain(`{{${key}}}`);
    expect(html).toContain("Angaben nach § 5 DDG.");
    expect(html).not.toContain("gelb markierten");
    expect(html).not.toContain("[");
    expect(page("de/datenschutz.html")).not.toContain(
      `{{${KEY.operatorName}}}`,
    );
  });
});

describe("markerLines", () => {
  it("nests blocks and puts every marker on its own line", () => {
    expect(
      markerLines(
        '<p><gelabber-if data-key="A"><gelabber-if data-key="B">x</gelabber-if></gelabber-if><gelabber-fragment data-name="terms"></gelabber-fragment></p>',
      ),
    ).toBe(
      "<p>\n<!--if:A-->\n\n<!--if:B-->\nx\n<!--end:B-->\n\n<!--end:A-->\n\n<!--gelabber:fragment terms-->\n</p>",
    );
  });

  it("refuses a block it cannot match", () => {
    expect(() => markerLines("</gelabber-if>")).toThrow(/unmatched/);
  });
});
