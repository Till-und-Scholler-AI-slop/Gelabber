// The frame around sign-in, invite and mail-link pages: the app icon and
// wordmark over one centred card, with the Living Room artwork behind it on
// wide screens. No app header; these pages are the front door. Under the
// card, the links to the legal pages this instance published.

import { Link, Outlet } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";

import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { language } from "../i18n/catalogs/language.ts";
import { useCurrentLocale } from "../i18n/locale.ts";
import {
  legalPageUrl,
  usePublicPages,
  type PublicPages,
} from "../public/pages.ts";
import { LEGAL_PAGES } from "../public/urls.ts";

import "./auth.css";

export function AuthLayout() {
  return (
    <AuthFrame>
      <Outlet />
    </AuthFrame>
  );
}

export function AuthFrame({ children }: { children: ReactNode }) {
  const pages = usePublicPages();
  const brand = (
    <>
      <img src="/icons/icon-192.png" alt="" width={40} height={40} />
      {/* i18n-ok */}Gelabber
    </>
  );
  return (
    <main className="auth-layout">
      <div className="auth-column">
        {pages.landing ? (
          // A full load, so nginx can serve the landing page at /.
          <a href="/" className="auth-brand">
            {brand}
          </a>
        ) : (
          <Link to="/" className="auth-brand">
            {brand}
          </Link>
        )}
        {children}
        <LegalLinks legal={pages.legal} />
      </div>
    </main>
  );
}

/**
 * The published legal pages, in the UI language where there is one. Plain
 * links: the pages are static documents. Nothing at all without one.
 */
function LegalLinks({ legal }: { legal: PublicPages["legal"] }) {
  const t = useT(auth);
  const locale = useCurrentLocale();
  const links = LEGAL_PAGES.flatMap((id) => {
    const urls = legal[id];
    const href = urls && legalPageUrl(urls, locale);
    return href ? [{ id, href }] : [];
  });
  if (links.length === 0) return null;
  return (
    <nav className="auth-legal" aria-label={t("legal.label")}>
      {links.map(({ id, href }) => (
        <a key={id} href={href}>
          {t(`legal.${id}`)}
        </a>
      ))}
    </nav>
  );
}

/** Shown above the heading when the visitor came from an invite link. */
export function InviteContextBanner() {
  const t = useT(auth);
  return (
    <p className="auth-banner auth-banner--invite">
      {t("layout.inviteBanner")}
    </p>
  );
}

let splashReplaced = false;

/**
 * While the app works out who is signed in: the same icon and wordmark as
 * the static splash in index.html. The first one takes over from that
 * splash at once; later ones fade in after 300 ms, so a quick answer never
 * flashes them.
 */
export function AppPlaceholder() {
  // The static splash's own text, so the hand-over does not change it.
  const t = useT(language);
  const [immediate] = useState(() => {
    const first = !splashReplaced;
    splashReplaced = true;
    return first;
  });
  return (
    <div
      className={`app-placeholder ${immediate ? "app-placeholder--immediate" : ""}`}
    >
      <img src="/icons/icon-192.png" alt="" width={48} height={48} />
      <strong>{/* i18n-ok */}Gelabber</strong>
      <span role="status">{t("splash.connecting")}</span>
    </div>
  );
}
