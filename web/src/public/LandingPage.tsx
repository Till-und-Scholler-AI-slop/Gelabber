// The public landing page: what Gelabber is, how to get on the hosted
// instance, and how to run your own. Drawn as a photocopied flyer pasted on
// a dark wall, its tear-off strips the actions
// (.impeccable/surfaces/web-src-public-landingpage-tsx.md). Static HTML;
// every link is a plain link.

import { translateIn } from "../i18n/catalog.ts";
import { language } from "../i18n/catalogs/language.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import { LOCALES, type Locale } from "../i18n/locale.ts";
import { KEY, value } from "./keys.ts";
import { Brand, PublicFrame } from "./PublicFrame.tsx";
import { If } from "./Template.tsx";
import { SELF_HOST_COMMANDS, textsIn } from "./text.ts";
import { feedbackUrl, LEGAL_PAGES, landingUrl, legalUrl } from "./urls.ts";

const FEATURES = ["lives", "open", "private"] as const;

const LEGAL_KEYS = {
  imprint: KEY.imprint,
  privacy: KEY.privacy,
  terms: KEY.terms,
} as const;

/** "Request an invite": a link, or mail with a subject, else nothing. */
function InviteButton({
  locale,
  className = "public-button",
}: {
  locale: Locale;
  className?: string;
}) {
  const text = textsIn(locale);
  const label = text("landing.cta.invite");
  const subject = encodeURIComponent(text("landing.invite.subject"));
  return (
    <>
      <If when={KEY.inviteUrl}>
        <a className={className} href={value(KEY.inviteUrl)}>
          {label}
        </a>
      </If>
      <If when={KEY.inviteEmail}>
        <a
          className={className}
          href={`mailto:${value(KEY.inviteEmail)}?subject=${subject}`}
        >
          {label}
        </a>
      </If>
    </>
  );
}

/** The cut line's scissors, drawn at the start of the tear-off strips. */
function Scissors() {
  return (
    <svg
      className="lp-scissors"
      viewBox="0 0 24 24"
      width="22"
      height="22"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="6" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M8.1 8.1 20 20M8.1 15.9 20 4" />
    </svg>
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
      <div className="lp">
        <header className="public-container public-bar">
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
        </header>
        <main>
          {/* The flyer on the wall, its tear-off strips the actions, and
              the app's picture taped up beside it. */}
          <div className="public-container lp-hero">
            <div className="lp-flyer lp-paper">
              <div className="lp-flyer__sheet">
                <h1>{text("landing.hero.title")}</h1>
                <p className="lp-lead">{text("landing.hero.lead")}</p>
                <p className="lp-note">{text("landing.hero.note")}</p>
              </div>
              <div className="lp-tabs">
                <Scissors />
                <InviteButton
                  locale={locale}
                  className="lp-tab lp-tab--invite"
                />
                <a className="lp-tab" href="/login">
                  {text("landing.nav.signIn")}
                </a>
                <a className="lp-tab" href="#self-host">
                  {text("landing.nav.selfHost")}
                </a>
                <a className="lp-tab" href={value(KEY.sourceUrl)}>
                  {text("landing.nav.source")}
                </a>
              </div>
            </div>
            <figure className="lp-print">
              <img
                src={`/images/landing/app-${locale}.png`}
                width={1280}
                height={800}
                alt={text("landing.screenshotAlt")}
              />
            </figure>
          </div>
          {/* The bill: the headliner first, then the rest. */}
          <div className="public-container lp-bill">
            {FEATURES.map((feature) => (
              <section key={feature}>
                <h2>{text(`landing.features.${feature}.title`)}</h2>
                <p>{text(`landing.features.${feature}.body`)}</p>
              </section>
            ))}
          </div>
          <div className="public-container lp-host">
            <section id="self-host" className="lp-sheet lp-paper">
              <h2>{text("landing.selfHost.title")}</h2>
              <p>{text("landing.selfHost.body")}</p>
              <pre>
                <code>{SELF_HOST_COMMANDS.join("\n")}</code>
              </pre>
              <a
                className="lp-button lp-button--outline"
                href={value(KEY.guideUrl)}
              >
                {text("landing.selfHost.guide")}
              </a>
            </section>
            <If when={KEY.contactEmail}>
              <section className="lp-card lp-paper">
                <h2>{text("landing.managed.title")}</h2>
                <p>{text("landing.managed.body")}</p>
                <a
                  className="lp-button"
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
              <If when={KEY.feedback}>
                <a href={feedbackUrl(locale)}>{text("feedback.link")}</a>
              </If>
              <a href={value(KEY.sourceUrl)}>{text("landing.nav.source")}</a>
              <span className="public-footer__languages">
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
              </span>
            </nav>
          </div>
        </footer>
      </div>
    </PublicFrame>
  );
}
