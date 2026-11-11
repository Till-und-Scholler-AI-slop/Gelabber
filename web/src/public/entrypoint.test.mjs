// The container's publishing step for the public pages
// (docker-entrypoint.d/50-gelabber-public-pages.sh), run on the templates
// render.tsx writes, as the image runs it at start.
/* global URL, process */
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { renderPublicTemplates } from "./render.tsx";

const script = fileURLToPath(
  new URL(
    "../../docker-entrypoint.d/50-gelabber-public-pages.sh",
    import.meta.url,
  ),
);

const OPERATOR = {
  GELABBER_OPERATOR_NAME: "Erika Mustermann",
  GELABBER_OPERATOR_STREET: "Musterstraße 1",
  GELABBER_OPERATOR_CITY: "12345 Musterstadt",
  GELABBER_OPERATOR_EMAIL: "kontakt@example.org",
};

let root;
let templates;

beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), "gelabber-public-"));
  templates = join(root, "templates");
  for (const { file, html } of renderPublicTemplates({
    stylesheet: "/assets/public-test.css",
  })) {
    mkdirSync(dirname(join(templates, file)), { recursive: true });
    writeFileSync(join(templates, file), html);
  }
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

/** Runs the script with these values and operator texts; what it wrote. */
function publish(values = {}, texts = {}) {
  const run = mkdtempSync(join(root, "run-"));
  const legal = join(run, "legal");
  mkdirSync(legal);
  for (const [name, text] of Object.entries(texts))
    writeFileSync(join(legal, name), text);
  const out = join(run, "out");
  const map = join(run, "nginx", "public", "landing.map");
  const env = {
    PATH: process.env.PATH,
    GELABBER_PUBLIC_TEMPLATES: templates,
    GELABBER_PUBLIC_OUT: out,
    GELABBER_LEGAL_DIR: legal,
    GELABBER_LANDING_MAP: map,
    ...values,
  };
  // execFileSync throws on any exit code but 0.
  const log = execFileSync("sh", [script], { env, encoding: "utf8" });
  const read = (file) =>
    existsSync(join(out, file)) ? readFileSync(join(out, file), "utf8") : null;
  const files = (dir) =>
    existsSync(join(out, dir)) ? readdirSync(join(out, dir)).sort() : [];
  return {
    log,
    read,
    files: [
      ...files("de").map((f) => `de/${f}`),
      ...files("en").map((f) => `en/${f}`),
    ],
    json: JSON.parse(read("public-pages.json")),
    map: existsSync(map) ? readFileSync(map, "utf8") : null,
  };
}

/** Nothing of the template's format is left on a published page. */
function expectFilled(html) {
  expect(html).not.toMatch(/\{\{|<!--if:|<!--end:|<!--gelabber:fragment/);
}

describe.skipIf(process.platform === "win32")("public pages at start", () => {
  it("publishes nothing by default, and keeps crawlers out", () => {
    const result = publish();
    expect(result.files).toEqual([]);
    expect(result.map).toBeNull();
    expect(result.json).toEqual({ version: 1, landing: false, legal: {} });
    expect(result.read("robots.txt")).toBe("User-agent: *\nDisallow: /\n");
    expect(result.read("sitemap.xml")).toBeNull();
    expect(result.log).toMatch(/the imprint page is not published/);
    expect(result.log).toMatch(/the privacy page is not published/);
    expect(result.log).toMatch(/the terms page is not published/);
    expect(result.log).toContain('(deploy/README.md, "Public pages")');
  });

  it("turns the landing page on with GELABBER_LANDING=TRUE", () => {
    const result = publish({ GELABBER_LANDING: "TRUE" });
    expect(result.files).toEqual(["de/index.html", "en/index.html"]);
    expect(result.map).toBe('"" 1;\n');
    expect(result.json).toEqual({ version: 1, landing: true, legal: {} });
    for (const file of result.files) expectFilled(result.read(file));
    expect(result.read("de/index.html")).toContain(
      'href="https://github.com/Till-und-Scholler-AI-slop/Gelabber"',
    );
    expect(result.read("en/index.html")).toContain(
      'href="https://github.com/Till-und-Scholler-AI-slop/Gelabber/blob/main/deploy/README.md"',
    );
    expect(result.log).toMatch(/published without an Impressum/);
  });

  it("leaves the Impressum out while a required value is missing", () => {
    const result = publish({
      ...OPERATOR,
      GELABBER_OPERATOR_STREET: "",
      GELABBER_OPERATOR_EMAIL: "  ",
      GELABBER_LANDING: "true",
    });
    expect(result.files).toEqual(["de/index.html", "en/index.html"]);
    expect(result.json.legal).toEqual({});
    expect(result.log).toMatch(
      /the imprint page is not published: missing GELABBER_OPERATOR_STREET GELABBER_OPERATOR_EMAIL /,
    );
    expect(result.read("de/index.html")).not.toContain("/de/impressum");
  });

  it("publishes the Impressum from the operator's values", () => {
    const result = publish(OPERATOR);
    expect(result.files).toEqual(["de/impressum.html", "en/imprint.html"]);
    const html = result.read("de/impressum.html");
    expectFilled(html);
    expect(html).toContain(
      "<p>Erika Mustermann<br/>Musterstraße 1<br/>12345 Musterstadt</p>",
    );
    expect(html).toContain(
      '<a href="mailto:kontakt@example.org">kontakt@example.org</a>',
    );
    // GELABBER_CONTENT_RESPONSIBLE and GELABBER_REPORT_EMAIL fall back.
    expect(html).toContain(
      "<p>Erika Mustermann, Musterstraße 1, 12345 Musterstadt</p>",
    );
    expect(html.match(/mailto:kontakt@example\.org/g)).toHaveLength(2);
    // No phone: the line is gone.
    expect(html).not.toContain("Telefon");
    expect(html).toContain('<div class="legal-text" lang="de">');
    expect(html).not.toContain('class="legal-notice"');
    expect(result.read("en/imprint.html")).not.toContain("Phone");
  });

  it("shows the phone, the responsible person and the report address when given", () => {
    const html = publish({
      ...OPERATOR,
      GELABBER_OPERATOR_PHONE: "+49 30 123456",
      GELABBER_CONTENT_RESPONSIBLE: "Max Muster, Weg 2, 54321 Ort",
      GELABBER_REPORT_EMAIL: "melden@example.org",
    }).read("de/impressum.html");
    expect(html).toContain("Telefon: +49 30 123456");
    expect(html).toContain("<p>Max Muster, Weg 2, 54321 Ort</p>");
    expect(html).toContain('href="mailto:melden@example.org"');
  });

  it("escapes every value, and never breaks on & < > \" ' \\", () => {
    const nasty = `A & B <i>"x"</i> 'y' \\n \\& & {{GELABBER_OPERATOR_EMAIL}}`;
    const result = publish({
      ...OPERATOR,
      GELABBER_OPERATOR_NAME: nasty,
      GELABBER_LANDING: "true",
      GELABBER_CONTACT_EMAIL: "a&b@example.org",
    });
    const html = result.read("en/imprint.html");
    expect(html).toContain(
      "<p>A &amp; B &lt;i&gt;&quot;x&quot;&lt;/i&gt; &#39;y&#39; \\n \\&amp; &amp; {{GELABBER_OPERATOR_EMAIL}}<br/>",
    );
    expect(html).not.toContain("<i>");
    expect(result.read("en/index.html")).toContain(
      'href="mailto:a&amp;b@example.org">Write to a&amp;b@example.org</a>',
    );
  });

  it("serves a text that exists in one language under both, with a notice", () => {
    const text = "<h2>Verantwortliche Stelle</h2>\n<p>{{KEPT}} &amp; so</p>\n";
    const result = publish({}, { "privacy.de.html": text });
    expect(result.files).toEqual(["de/datenschutz.html", "en/privacy.html"]);
    const en = result.read("en/privacy.html");
    expect(en).toContain(
      '<p class="legal-notice">This text is only available in German.</p>',
    );
    // The operator's text goes in as it is.
    expect(en).toContain(`<div class="legal-text" lang="de">\n${text}</div>`);
    const de = result.read("de/datenschutz.html");
    expect(de).not.toContain('class="legal-notice"');
    expect(de).toContain(`<div class="legal-text" lang="de">\n${text}</div>`);
    expect(result.json.legal).toEqual({
      privacy: { de: "/de/datenschutz", en: "/en/privacy" },
    });
  });

  it("takes each language's own text when both exist, and skips empty files", () => {
    const result = publish(
      {},
      {
        "terms.de.html": "<p>Deutsch</p>\n",
        "terms.en.html": "<p>English</p>\n",
        "privacy.en.html": "",
      },
    );
    expect(result.files).toEqual([
      "de/nutzungsbedingungen.html",
      "en/terms.html",
    ]);
    expect(result.read("de/nutzungsbedingungen.html")).toContain(
      '<div class="legal-text" lang="de">\n<p>Deutsch</p>\n</div>',
    );
    expect(result.read("en/terms.html")).toContain(
      '<div class="legal-text" lang="en">\n<p>English</p>\n</div>',
    );
    expect(result.log).toMatch(/the privacy page is not published/);
  });

  it("links only the published legal pages", () => {
    const result = publish(
      { ...OPERATOR, GELABBER_LANDING: "true" },
      { "terms.en.html": "<p>Terms</p>\n" },
    );
    const landing = result.read("de/index.html");
    expect(landing).toContain('<a href="/de/impressum">');
    expect(landing).toContain('<a href="/de/nutzungsbedingungen">');
    expect(landing).not.toContain("/de/datenschutz");
    const terms = result.read("de/nutzungsbedingungen.html");
    expectFilled(terms);
    expect(terms).not.toContain("/de/datenschutz");
    expect(terms).toContain(
      '<p class="legal-notice">Dieser Text liegt nur auf Englisch vor.</p>',
    );
  });

  it("leaves both invite buttons and the managed column out with no contact", () => {
    const html = publish({ GELABBER_LANDING: "true" }).read("en/index.html");
    expect(html).not.toContain("Request an invite");
    expect(html).not.toContain("mailto:");
    expect(html).not.toContain("run by us");
  });

  it("points the invite buttons at GELABBER_INVITE_REQUEST, or the contact address", () => {
    const count = (html, text) => html.split(text).length - 1;
    const url = publish({
      GELABBER_LANDING: "true",
      GELABBER_INVITE_REQUEST: "https://example.org/invite?a=1&b=2",
      GELABBER_CONTACT_EMAIL: "hallo@example.org",
    }).read("en/index.html");
    expect(count(url, 'href="https://example.org/invite?a=1&amp;b=2"')).toBe(2);
    expect(url).not.toContain("subject=");
    expect(url).toContain("Write to hallo@example.org");

    const email = publish({
      GELABBER_LANDING: "true",
      GELABBER_INVITE_REQUEST: "invite@example.org",
    }).read("de/index.html");
    expect(
      count(email, 'href="mailto:invite@example.org?subject=Einladung%20f'),
    ).toBe(2);
    expect(email).not.toContain("Schreib an");

    const contact = publish({
      GELABBER_LANDING: "true",
      GELABBER_CONTACT_EMAIL: "hallo@example.org",
    }).read("en/index.html");
    expect(
      count(
        contact,
        'href="mailto:hallo@example.org?subject=Gelabber%20invite"',
      ),
    ).toBe(2);

    const wrong = publish({
      GELABBER_LANDING: "true",
      GELABBER_INVITE_REQUEST: "javascript:alert(1)",
    });
    expect(wrong.read("en/index.html")).not.toContain("Request an invite");
    expect(wrong.log).toMatch(/GELABBER_INVITE_REQUEST is neither/);
  });

  it("uses no link value that is not an address or an http(s) URL", () => {
    const result = publish({
      ...OPERATOR,
      GELABBER_LANDING: "true",
      GELABBER_CONTACT_EMAIL: "x@y.de?bcc=evil@z",
      GELABBER_REPORT_EMAIL: "melden@example.org?cc=a@b",
      GELABBER_SOURCE_URL: "javascript:alert(1)",
      GELABBER_OPERATOR_GUIDE_URL: "http://guide.example.org/",
      APP_PUBLIC_URL: "gelabber.example.org",
    });
    const landing = result.read("en/index.html");
    expect(landing).not.toContain("javascript:");
    expect(landing).toContain(
      'href="https://github.com/Till-und-Scholler-AI-slop/Gelabber"',
    );
    expect(landing).toContain('href="http://guide.example.org/"');
    // The contact address is neither the invite target nor "Write to".
    expect(landing).not.toContain("mailto:");
    expect(landing).not.toContain("Request an invite");
    expect(landing).not.toContain('rel="canonical"');
    expect(result.read("sitemap.xml")).toBeNull();
    const imprint = result.read("en/imprint.html");
    expect(imprint).not.toContain("?cc=");
    expect(imprint.match(/mailto:kontakt@example\.org/g)).toHaveLength(2);
    for (const name of [
      "GELABBER_CONTACT_EMAIL is not an email address",
      "GELABBER_REPORT_EMAIL is not an email address",
      "GELABBER_SOURCE_URL is not an http(s):// URL",
      "APP_PUBLIC_URL is not an http(s):// URL",
    ])
      expect(result.log).toContain(name);
    expect(result.log).not.toContain("GELABBER_OPERATOR_GUIDE_URL");

    const operator = publish({
      ...OPERATOR,
      GELABBER_OPERATOR_EMAIL: "kontakt@example.org?bcc=a@b",
    });
    expect(operator.files).toEqual([]);
    expect(operator.log).toMatch(
      /the imprint page is not published: missing GELABBER_OPERATOR_EMAIL /,
    );
  });

  it("links no legal page that could not be written", () => {
    // A broken privacy template makes its render fail after the flags are set.
    const broken = mkdtempSync(join(root, "broken-templates-"));
    for (const lang of ["de", "en"]) {
      mkdirSync(join(broken, lang));
      for (const file of readdirSync(join(templates, lang)))
        writeFileSync(
          join(broken, lang, file),
          readFileSync(join(templates, lang, file)),
        );
    }
    writeFileSync(join(broken, "en", "privacy.html"), "<!--end:X-->\n");
    const result = publish(
      { ...OPERATOR, GELABBER_LANDING: "true", GELABBER_PUBLIC_TEMPLATES: broken },
      { "privacy.en.html": "<p>p</p>", "terms.en.html": "<p>t</p>" },
    );
    expect(result.log).toMatch(/cannot write the privacy page/);
    expect(result.files).toEqual([
      "de/impressum.html",
      "de/index.html",
      "de/nutzungsbedingungen.html",
      "en/imprint.html",
      "en/index.html",
      "en/terms.html",
    ]);
    for (const file of result.files) {
      expect(result.read(file)).not.toContain("/de/datenschutz");
      expect(result.read(file)).not.toContain("/en/privacy");
    }
    expect(result.read("de/index.html")).toContain(
      '<a href="/de/nutzungsbedingungen">',
    );
    expect(Object.keys(result.json.legal)).toEqual(["imprint", "terms"]);
  });

  it("describes what it published in public-pages.json, robots.txt and sitemap.xml", () => {
    const result = publish(
      {
        ...OPERATOR,
        GELABBER_LANDING: "true",
        APP_PUBLIC_URL: "https://gelabber.example.org/",
      },
      { "privacy.en.html": "<p>p</p>", "terms.de.html": "<p>t</p>" },
    );
    expect(result.json).toEqual({
      version: 1,
      landing: true,
      legal: {
        imprint: { de: "/de/impressum", en: "/en/imprint" },
        privacy: { de: "/de/datenschutz", en: "/en/privacy" },
        terms: { de: "/de/nutzungsbedingungen", en: "/en/terms" },
      },
    });
    expect(result.files).toHaveLength(8);
    for (const file of result.files) expectFilled(result.read(file));
    expect(result.read("en/index.html")).toContain(
      '<link rel="canonical" href="https://gelabber.example.org/en/">',
    );
    const robots = result.read("robots.txt").split("\n");
    for (const line of [
      "User-agent: *",
      "Allow: /$",
      "Allow: /de/",
      "Allow: /en/",
      "Allow: /de/datenschutz",
      "Allow: /en/terms",
      "Disallow: /login",
      "Disallow: /invite/",
      "Disallow: /api/",
      "Disallow: /ws",
      "Sitemap: https://gelabber.example.org/sitemap.xml",
    ])
      expect(robots).toContain(line);
    expect(robots).not.toContain("Disallow: /");
    const sitemap = result.read("sitemap.xml");
    expect(sitemap).toContain(
      '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">',
    );
    expect(
      [...sitemap.matchAll(/<loc>([^<]*)<\/loc>/g)].map(([, loc]) => loc),
    ).toEqual([
      "https://gelabber.example.org/de/",
      "https://gelabber.example.org/en/",
      "https://gelabber.example.org/de/impressum",
      "https://gelabber.example.org/en/imprint",
      "https://gelabber.example.org/de/datenschutz",
      "https://gelabber.example.org/en/privacy",
      "https://gelabber.example.org/de/nutzungsbedingungen",
      "https://gelabber.example.org/en/terms",
    ]);
  });

  it("leaves the absolute links and the sitemap out without APP_PUBLIC_URL", () => {
    const result = publish({ GELABBER_LANDING: "true" });
    expect(result.read("en/index.html")).not.toContain('rel="canonical"');
    expect(result.read("robots.txt")).not.toContain("Sitemap:");
    expect(result.read("sitemap.xml")).toBeNull();
  });

  it("clears what the last start published", () => {
    const run = mkdtempSync(join(root, "again-"));
    const env = {
      PATH: process.env.PATH,
      GELABBER_PUBLIC_TEMPLATES: templates,
      GELABBER_PUBLIC_OUT: join(run, "out"),
      GELABBER_LEGAL_DIR: join(run, "legal"),
      GELABBER_LANDING_MAP: join(run, "landing.map"),
      APP_PUBLIC_URL: "https://gelabber.example.org",
      ...OPERATOR,
    };
    execFileSync("sh", [script], {
      env: { ...env, GELABBER_LANDING: "true" },
    });
    expect(existsSync(join(run, "landing.map"))).toBe(true);
    execFileSync("sh", [script], { env });
    expect(existsSync(join(run, "landing.map"))).toBe(false);
    expect(existsSync(join(run, "out", "de", "index.html"))).toBe(false);
    expect(existsSync(join(run, "out", "sitemap.xml"))).toBe(false);
    expect(existsSync(join(run, "out", "de", "impressum.html"))).toBe(true);
  });

  it("never fails the container, even without templates or a writable folder", () => {
    const run = mkdtempSync(join(root, "broken-"));
    writeFileSync(join(run, "file"), "");
    const env = {
      PATH: process.env.PATH,
      GELABBER_PUBLIC_TEMPLATES: join(run, "none"),
      GELABBER_LEGAL_DIR: join(run, "none"),
      GELABBER_LANDING_MAP: join(run, "landing.map"),
      GELABBER_LANDING: "true",
      ...OPERATOR,
    };
    const log = execFileSync("sh", [script], {
      env: { ...env, GELABBER_PUBLIC_OUT: join(run, "out") },
      encoding: "utf8",
    });
    expect(log).toMatch(/no landing page template/);
    expect(log).toMatch(
      /the imprint page is not published: its templates are missing/,
    );
    expect(existsSync(join(run, "landing.map"))).toBe(false);
    expect(
      JSON.parse(readFileSync(join(run, "out", "public-pages.json"), "utf8")),
    ).toEqual({ version: 1, landing: false, legal: {} });
    // A file where the folder should be.
    expect(
      execFileSync("sh", [script], {
        env: { ...env, GELABBER_PUBLIC_OUT: join(run, "file", "out") },
        encoding: "utf8",
      }),
    ).toMatch(/cannot write/);
  });
});
