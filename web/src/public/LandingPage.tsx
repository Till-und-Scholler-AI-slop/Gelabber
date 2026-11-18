// The public landing page: what Gelabber is, how to get on the hosted
// instance, and how to run your own. Drawn as a LAN party: a row of monitors
// with tape name labels, two of them live
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

/** Ada's Live on the left monitor: the self-host commands' first run.
 * Terminal output, the same in both languages. */
const TERMINAL = [
  ["$ ", "docker compose up"],
  ["", "[+] Running 10/10"],
  ["", " Network compose_default", "Created"],
  ["", " Container compose-postgres-1", "Healthy"],
  ["", " Container compose-redis-1", "Healthy"],
  ["", " Container compose-minio-1", "Healthy"],
  ["", " Container compose-minio-init-1", "Exited"],
  ["", " Container compose-coturn-1", "Started"],
  ["", " Container compose-api-1", "Started"],
  ["", " Container compose-media-1", "Started"],
  ["", " Container compose-web-1", "Started"],
  ["", " Container compose-proxy-1", "Started"],
] as const;

/** A strip of masking tape with a name on it, as on a LAN party monitor. */
function NameTape({ name }: { name: string }) {
  return <span className="lp-tape">{name}</span>;
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
          <div className="public-container lp-hero">
            <h1>{text("landing.hero.title")}</h1>
            <div className="lp-intro">
              <p className="lp-lead">{text("landing.hero.lead")}</p>
              <div className="lp-actions">
                <InviteButton
                  locale={locale}
                  className="lp-button lp-button--primary"
                />
                <a className="lp-button lp-button--ghost" href="#self-host">
                  {text("landing.nav.selfHost")}
                </a>
              </div>
              <p className="lp-note">{text("landing.hero.note")}</p>
            </div>
          </div>
          {/* A LAN party table: Ada and Jonas are live, Lena's own screen
              in the middle is the real app. The side screens are made up. */}
          <div className="lp-desk">
            <div className="public-container lp-monitors">
              <div className="lp-monitor lp-monitor--left" aria-hidden="true">
                <div className="lp-monitor__frame">
                  <div className="lp-monitor__screen lp-term">
                    {TERMINAL.map(([prompt, line, state]) => (
                      <span key={line}>
                        {prompt ? <b>{prompt}</b> : null}
                        {line}
                        {state ? <em> {state}</em> : null}
                      </span>
                    ))}
                    <span className="lp-live">
                      {text("landing.monitors.live")}
                    </span>
                  </div>
                </div>
                <NameTape name="Ada" /* i18n-ok */ />
              </div>
              <figure className="lp-monitor lp-monitor--main">
                <div className="lp-monitor__frame">
                  <div className="lp-monitor__screen">
                    <img
                      src={`/images/landing/app-${locale}.png`}
                      width={1280}
                      height={800}
                      alt={text("landing.screenshotAlt")}
                    />
                  </div>
                </div>
                <NameTape name="Lena" /* i18n-ok */ />
              </figure>
              <div className="lp-monitor lp-monitor--right" aria-hidden="true">
                <div className="lp-monitor__frame">
                  <div className="lp-monitor__screen">
                    <img
                      src="/images/landing/lan-map.svg"
                      width={1280}
                      height={800}
                      alt=""
                    />
                    <span className="lp-live">
                      {text("landing.monitors.live")}
                    </span>
                  </div>
                </div>
                <NameTape name="Jonas" /* i18n-ok */ />
              </div>
            </div>
          </div>
          {/* The stations: each feature under its tape label, the
              headliner first. */}
          <div className="public-container lp-bill">
            {FEATURES.map((feature) => (
              <section key={feature}>
                <h2>{text(`landing.features.${feature}.title`)}</h2>
                <p>{text(`landing.features.${feature}.body`)}</p>
              </section>
            ))}
          </div>
          <div className="public-container lp-host">
            <section id="self-host" className="lp-panel">
              <h2>{text("landing.selfHost.title")}</h2>
              <p>{text("landing.selfHost.body")}</p>
              <pre>
                <code>{SELF_HOST_COMMANDS.join("\n")}</code>
              </pre>
              <a
                className="lp-button lp-button--ghost"
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
