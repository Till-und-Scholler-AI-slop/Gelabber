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
  "/de/feedback",
  "/en/feedback",
  "/de/feedback/gesendet",
  "/de/feedback/danke",
  "/de/feedback/abgelaufen",
  "/de/feedback/fehler",
  "/en/feedback/sent",
  "/en/feedback/thanks",
  "/en/feedback/expired",
  "/en/feedback/error",
];

const FEEDBACK_FILES = [
  "de/feedback.html",
  "de/feedback/abgelaufen.html",
  "de/feedback/danke.html",
  "de/feedback/fehler.html",
  "de/feedback/gesendet.html",
  "en/feedback.html",
  "en/feedback/error.html",
  "en/feedback/expired.html",
  "en/feedback/sent.html",
  "en/feedback/thanks.html",
];

const LEGAL_BLOCKS: Record<string, string> = {
  "/de/impressum": KEY.imprint,
  "/en/imprint": KEY.imprint,
  "/de/datenschutz": KEY.privacy,
  "/en/privacy": KEY.privacy,
  "/de/nutzungsbedingungen": KEY.terms,
  "/en/terms": KEY.terms,
  "/de/feedback": KEY.feedback,
  "/en/feedback": KEY.feedback,
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
  it("are the landing page, the three legal pages and the feedback pages in both languages", () => {
    expect(templates.map((t) => t.file).sort()).toEqual(
      [
        "de/datenschutz.html",
        "de/impressum.html",
        "de/index.html",
        "de/nutzungsbedingungen.html",
        "en/imprint.html",
        "en/index.html",
        "en/privacy.html",
        "en/terms.html",
        ...FEEDBACK_FILES,
      ].sort(),
    );
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
    ({ file, html }) => {
      // The feedback pages are published only together, so they link each
      // other freely.
      const blocks = FEEDBACK_FILES.includes(file)
        ? Object.fromEntries(
            Object.entries(LEGAL_BLOCKS).filter(
              ([, key]) => key !== KEY.feedback,
            ),
          )
        : LEGAL_BLOCKS;
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
          if (blocks[href]) expect(keys, href).toContain(blocks[href]);
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

describe("feedback pages", () => {
  const form = (locale: "de" | "en") => page(`${locale}/feedback.html`);
  const attributes = (html: string, tag: string, name: string) =>
    html.match(new RegExp(`<${tag}\\b[^>]*\\bname="${name}"[^>]*>`))?.[0] ?? "";

  it.each(FEEDBACK_FILES)("%s is kept out of search engines", (file) => {
    const html = page(file);
    expect(html).toContain('<meta name="robots" content="noindex">');
    expect(html).not.toContain('hreflang="x-default"');
    expect(html).toContain('<header class="legal-top">');
    expect(html).toContain('<a href="/login">');
  });

  it("is noindex only there", () => {
    for (const { file, html } of templates)
      if (!FEEDBACK_FILES.includes(file))
        expect(html, file).not.toContain('name="robots"');
  });

  it.each(["de", "en"] as const)(
    "the %s form posts its five fields to the API",
    (locale) => {
      const html = form(locale);
      const forms = html.match(/<form\b[^>]*>/g) ?? [];
      expect(forms).toHaveLength(1);
      for (const attribute of [
        'method="post"',
        'action="/api/public/feedback"',
        'accept-charset="utf-8"',
      ])
        expect(forms[0]).toContain(attribute);
      expect(html).toContain(
        `<input type="hidden" name="lang" value="${locale}"/>`,
      );
      const kinds = [
        ...html.matchAll(/<input type="radio" name="kind"[^>]*>/g),
      ].map(([input]) => input);
      expect(
        kinds.map((input) => input.match(/value="([a-z]+)"/)?.[1]),
      ).toEqual(["idea", "problem", "praise"]);
      expect(kinds.map((input) => input.includes('checked=""'))).toEqual([
        true,
        false,
        false,
      ]);
      const text = attributes(html, "textarea", "text");
      expect(text).toContain('id="feedback-text"');
      expect(text).toContain("required");
      expect(text).toContain('maxLength="4000"');
      expect(text).toContain('aria-describedby="feedback-text-hint"');
      const email = attributes(html, "input", "email");
      expect(email).toContain('type="email"');
      expect(email).toContain("required");
      expect(email).toContain('maxLength="254"');
      expect(email).toContain('autoComplete="email"');
      expect(html).toContain('<label for="feedback-text">');
      expect(html).toContain('<label for="feedback-email">');
      expect(html).toContain('id="feedback-text-hint"');
      expect(html).toContain('id="feedback-email-hint"');
      expect(html).toContain('<button type="submit" class="public-button">');
      expect(decode(html)).toContain(publicPages[locale]["feedback.lead"]);
    },
  );

  it("hides the honeypot from people, not from bots", () => {
    const html = form("de");
    expect(html).toContain('<div class="feedback-hp" aria-hidden="true">');
    const website = attributes(html, "input", "website");
    expect(website).toContain('type="text"');
    expect(website).toContain('tabindex="-1"');
    expect(website).toContain('autoComplete="off"');
    expect(website).not.toContain("hidden");
    // public.css moves it off-screen (publicCss.test.mjs).
  });

  it("links the privacy page only while it is published", () => {
    for (const locale of ["de", "en"] as const) {
      const privacy = locale === "de" ? "/de/datenschutz" : "/en/privacy";
      const lines = blockLines(form(locale)).filter(({ line }) =>
        line.includes(`href="${privacy}"`),
      );
      expect(lines).toHaveLength(1);
      expect(lines[0].keys).toEqual([KEY.privacy]);
    }
  });

  it("links the form from the landing footer and the legal nav, inside PAGE_FEEDBACK", () => {
    for (const file of [
      "de/index.html",
      "en/index.html",
      "de/impressum.html",
      "en/terms.html",
    ]) {
      const href = `/${file.slice(0, 2)}/feedback`;
      const lines = blockLines(page(file)).filter(({ line }) =>
        line.includes(`href="${href}"`),
      );
      expect(lines, file).toHaveLength(1);
      expect(lines[0].keys, file).toEqual([KEY.feedback]);
    }
    expect(page("de/index.html")).toMatch(
      /<footer[\s\S]*href="\/de\/feedback"[\s\S]*<\/footer>/,
    );
    expect(page("de/datenschutz.html")).toMatch(
      /<nav class="legal-nav"[\s\S]*<li><a href="\/de\/feedback">Feedback<\/a><\/li>[\s\S]*<\/nav>/,
    );
  });

  it.each([
    ["de/feedback/gesendet.html", "sent", "/de/feedback"],
    ["en/feedback/sent.html", "sent", "/en/feedback"],
    ["de/feedback/abgelaufen.html", "expired", "/de/feedback"],
    ["en/feedback/expired.html", "expired", "/en/feedback"],
    ["de/feedback/fehler.html", "error", "/de/feedback"],
    ["en/feedback/error.html", "error", "/en/feedback"],
    ["de/feedback/danke.html", "thanks", null],
    ["en/feedback/thanks.html", "thanks", null],
  ] as const)("%s says what happened", (file, state, back) => {
    const locale = file.slice(0, 2) as "de" | "en";
    const html = decode(page(file));
    const texts = publicPages[locale];
    expect(html).toContain(`<h1>${texts[`feedback.${state}.title`]}</h1>`);
    expect(html).toContain(
      texts[`feedback.${state}.body`].replace(/<\/?link>/g, "").split(".")[0],
    );
    expect(html).not.toContain("<form");
    const links = anchors(html).filter((href) => href.includes("/feedback"));
    // The language switch goes to the same page in the other language.
    const other = links.filter((href) => !href.startsWith(`/${locale}/`));
    expect(other).toHaveLength(1);
    if (back) expect(links).toContain(back);
    else
      expect(links.filter((href) => href.startsWith(`/${locale}/`))).toEqual(
        [],
      );
  });

  it("switches the language to the same feedback page", () => {
    expect(page("de/feedback/gesendet.html")).toContain(
      '<a href="/en/feedback/sent" hrefLang="en" lang="en">',
    );
    expect(page("en/feedback.html")).toContain(
      '<a href="/de/feedback" hrefLang="de" lang="de">',
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
