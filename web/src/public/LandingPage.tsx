// The public landing page (docs/design/v0.10/hosted-landing.webp): what
// Gelabber is, how to get on the hosted instance, and how to run your own.
// Static HTML; every link is a plain link.

import { translateIn } from "../i18n/catalog.ts";
import { language } from "../i18n/catalogs/language.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import { LOCALES, type Locale } from "../i18n/locale.ts";
import { KEY, value } from "./keys.ts";
import { Brand, PublicFrame } from "./PublicFrame.tsx";
import { If } from "./Template.tsx";
import { SELF_HOST_COMMANDS, textsIn } from "./text.ts";
import { LEGAL_PAGES, landingUrl, legalUrl } from "./urls.ts";

const FEATURES = ["lives", "open", "private"] as const;

const LEGAL_KEYS = {
  imprint: KEY.imprint,
  privacy: KEY.privacy,
  terms: KEY.terms,
} as const;

/** "Request an invite": a link, or mail with a subject, else nothing. */
function InviteButton({ locale }: { locale: Locale }) {
  const text = textsIn(locale);
  const label = text("landing.cta.invite");
  const subject = encodeURIComponent(text("landing.invite.subject"));
  return (
    <>
      <If when={KEY.inviteUrl}>
        <a className="public-button" href={value(KEY.inviteUrl)}>
          {label}
        </a>
      </If>
      <If when={KEY.inviteEmail}>
        <a
          className="public-button"
          href={`mailto:${value(KEY.inviteEmail)}?subject=${subject}`}
        >
          {label}
        </a>
      </If>
    </>
  );
}

export function LandingPage({
  locale,
  stylesheet,
}: {
  locale: Locale;
  stylesheet: string;
}) {
  const text = textsIn(locale);
  return (
    <PublicFrame
      locale={locale}
      title={text("meta.landingTitle")}
      description={text("landing.hero.lead")}
      stylesheet={stylesheet}
      paths={{ de: landingUrl("de"), en: landingUrl("en") }}
      defaultPath="/"
      image={`/images/landing/app-${locale}.png`}
    >
      <header className="landing-hero">
        <div className="public-container public-bar">
          <Brand />
          <nav className="public-nav" aria-label={text("landing.nav.label")}>
            <a className="public-nav__wide" href="#self-host">
              {text("landing.nav.selfHost")}
            </a>
            <a className="public-nav__wide" href={value(KEY.sourceUrl)}>
              {text("landing.nav.source")}
            </a>
            <a href="/login">{text("landing.nav.signIn")}</a>
            <InviteButton locale={locale} />
          </nav>
        </div>
        <div className="public-container landing-intro">
          <h1>{text("landing.hero.title")}</h1>
          <p className="landing-lead">{text("landing.hero.lead")}</p>
          <div className="landing-actions">
            <InviteButton locale={locale} />
            <a
              className="public-button public-button--secondary"
              href="#self-host"
            >
              {text("landing.nav.selfHost")}
            </a>
          </div>
          <p className="landing-note">{text("landing.hero.note")}</p>
        </div>
      </header>
      <main className="public-container landing-main">
        <figure className="landing-shot">
          <img
            src={`/images/landing/app-${locale}.png`}
            width={1280}
            height={800}
            alt={text("landing.screenshotAlt")}
          />
        </figure>
        <div className="landing-features">
          {FEATURES.map((feature) => (
            <section key={feature}>
              <h2>{text(`landing.features.${feature}.title`)}</h2>
              <p>{text(`landing.features.${feature}.body`)}</p>
            </section>
          ))}
        </div>
        <div className="landing-host">
          <section id="self-host">
            <h2>{text("landing.selfHost.title")}</h2>
            <p>{text("landing.selfHost.body")}</p>
            <pre>
              <code>{SELF_HOST_COMMANDS.join("\n")}</code>
            </pre>
            <a
              className="public-button public-button--secondary"
              href={value(KEY.guideUrl)}
            >
              {text("landing.selfHost.guide")}
            </a>
          </section>
          <If when={KEY.contactEmail}>
            <section>
              <h2>{text("landing.managed.title")}</h2>
              <p>{text("landing.managed.body")}</p>
              <a
                className="public-button"
                href={`mailto:${value(KEY.contactEmail)}`}
              >
                {translateIn(locale, publicPages, "landing.managed.write", {
                  email: value(KEY.contactEmail),
                })}
              </a>
            </section>
          </If>
        </div>
      </main>
      <footer className="public-footer">
        <div className="public-container public-footer__inner">
          <p>{text("landing.footer.licence")}</p>
          <nav aria-label={text("landing.footer.label")}>
            {LEGAL_PAGES.map((page) => (
              <If key={page} when={LEGAL_KEYS[page]}>
                <a href={legalUrl(locale, page)}>{text(`legal.${page}`)}</a>
              </If>
            ))}
            <a href={value(KEY.sourceUrl)}>{text("landing.nav.source")}</a>
            {LOCALES.map((lang) => (
              <a
                key={lang}
                href={landingUrl(lang)}
                hrefLang={lang}
                lang={lang}
                aria-current={lang === locale ? "page" : undefined}
              >
                {translateIn(lang, language, `option.${lang}`)}
              </a>
            ))}
          </nav>
        </div>
      </footer>
    </PublicFrame>
  );
}
