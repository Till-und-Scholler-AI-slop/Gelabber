// The public pages as templates, in German and English, written into
// dist/public-pages/templates/ by scripts/public-pages/prerender.mjs. The
// web container fills them in at start (keys.ts) and publishes only what
// the operator supplied.
//
// Template format, one marker per line so a line-based awk can read it:
//   {{KEY}}                          a value, HTML-escaped when filled in
//   <!--if:KEY--> … <!--end:KEY-->   a block, dropped when KEY is empty
//   <!--gelabber:fragment NAME-->    the operator's text for page NAME

import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LOCALES } from "../i18n/locale.ts";
import { FeedbackPage } from "./FeedbackPage.tsx";
import { ImprintBody } from "./ImprintBody.tsx";
import { LandingPage } from "./LandingPage.tsx";
import { LegalPage } from "./LegalPage.tsx";
import {
  FEEDBACK_SLUGS,
  FEEDBACK_STATES,
  LEGAL_PAGES,
  legalSlug,
} from "./urls.ts";

export type PublicTemplate = { file: string; html: string };

/** Turns the stand-ins of Template.tsx into the template's marker lines. */
export function markerLines(markup: string): string {
  const open: string[] = [];
  return markup.replace(
    /<gelabber-if data-key="([A-Z_]+)">|<\/gelabber-if>|<gelabber-fragment data-name="([a-z]+)"><\/gelabber-fragment>/g,
    (tag, key: string | undefined, fragment: string | undefined) => {
      if (fragment) return `\n<!--gelabber:fragment ${fragment}-->\n`;
      if (key) {
        open.push(key);
        return `\n<!--if:${key}-->\n`;
      }
      const closed = open.pop();
      if (!closed) throw new Error(`public pages: unmatched ${tag}`);
      return `\n<!--end:${closed}-->\n`;
    },
  );
}

function document(element: ReactElement): string {
  return `<!doctype html>\n${markerLines(renderToStaticMarkup(element))}\n`;
}

/**
 * Every public page in both languages: `de/index.html`,
 * `de/impressum.html` … `en/terms.html`, and the feedback form with its
 * four answer pages, `de/feedback.html` and `de/feedback/gesendet.html` …
 * `en/feedback/error.html`. `stylesheet` is the built public.css.
 */
export function renderPublicTemplates({
  stylesheet,
}: {
  stylesheet: string;
}): PublicTemplate[] {
  return LOCALES.flatMap((locale) => [
    {
      file: `${locale}/index.html`,
      html: document(<LandingPage locale={locale} stylesheet={stylesheet} />),
    },
    ...LEGAL_PAGES.map((page) => ({
      file: `${locale}/${legalSlug(locale, page)}.html`,
      html: document(
        <LegalPage locale={locale} page={page} stylesheet={stylesheet}>
          {page === "imprint" ? <ImprintBody locale={locale} /> : null}
        </LegalPage>,
      ),
    })),
    ...FEEDBACK_STATES.map((state) => ({
      file: `${locale}/${FEEDBACK_SLUGS[locale][state]}.html`,
      html: document(
        <FeedbackPage locale={locale} state={state} stylesheet={stylesheet} />,
      ),
    })),
  ]);
}
