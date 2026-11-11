// A legal page (docs/design/v0.10/hosted-imprint.png): a top bar with the
// brand, the other language and sign-in; the published legal pages on the
// left; the text on the right. Below 800 px the list sits above the text.

import type { ReactNode } from "react";

import { translateIn } from "../i18n/catalog.ts";
import { language } from "../i18n/catalogs/language.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import type { Locale } from "../i18n/locale.ts";
import { KEY, value } from "./keys.ts";
import { Brand, PublicFrame } from "./PublicFrame.tsx";
import { FragmentSlot, If } from "./Template.tsx";
import { textsIn } from "./text.ts";
import {
  LEGAL_PAGES,
  legalUrl,
  otherLocale,
  type LegalPageId,
} from "./urls.ts";

const PAGE_KEYS = {
  imprint: KEY.imprint,
  privacy: KEY.privacy,
  terms: KEY.terms,
} as const;

export function LegalPage({
  locale,
  page,
  stylesheet,
  children,
}: {
  locale: Locale;
  page: LegalPageId;
  stylesheet: string;
  children: ReactNode;
}) {
  const text = textsIn(locale);
  const name = text(`legal.${page}`);
  const other = otherLocale(locale);
  return (
    <PublicFrame
      locale={locale}
      title={translateIn(locale, publicPages, "meta.pageTitle", { page: name })}
      description={translateIn(locale, publicPages, "meta.legalDescription", {
        page: name,
      })}
      stylesheet={stylesheet}
      paths={{ de: legalUrl("de", page), en: legalUrl("en", page) }}
    >
      <header className="legal-top">
        <div className="public-container public-bar">
          <Brand />
          <div className="public-nav">
            <a href={legalUrl(other, page)} hrefLang={other} lang={other}>
              {translateIn(other, language, `option.${other}`)}
            </a>
            <a href="/login">{text("legal.signIn")}</a>
          </div>
        </div>
      </header>
      <div className="public-container legal-layout">
        <nav className="legal-nav" aria-label={text("legal.navLabel")}>
          <ul>
            {LEGAL_PAGES.map((item) => (
              <If key={item} when={PAGE_KEYS[item]}>
                <li>
                  <a
                    href={legalUrl(locale, item)}
                    aria-current={item === page ? "page" : undefined}
                  >
                    {text(`legal.${item}`)}
                  </a>
                </li>
              </If>
            ))}
          </ul>
        </nav>
        <main className="legal-content">
          <h1>{name}</h1>
          {children}
          <If when={KEY.onlyOtherLanguage}>
            <p className="legal-notice">{text("legal.onlyOtherLanguage")}</p>
          </If>
          <div className="legal-text" lang={value(KEY.fragmentLang)}>
            <FragmentSlot name={page} />
          </div>
        </main>
      </div>
    </PublicFrame>
  );
}
