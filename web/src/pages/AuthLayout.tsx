// The frame around sign-in, invite and mail-link pages: the app icon and
// wordmark over one centred card, with the Living Room artwork behind it on
// wide screens. No app header; these pages are the front door.

import { Link, Outlet } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";

import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { language } from "../i18n/catalogs/language.ts";

import "./auth.css";

export function AuthLayout() {
  return (
    <AuthFrame>
      <Outlet />
    </AuthFrame>
  );
}

export function AuthFrame({ children }: { children: ReactNode }) {
  return (
    <main className="auth-layout">
      <div className="auth-column">
        <Link to="/" className="auth-brand">
          <img src="/icons/icon-192.png" alt="" width={40} height={40} />
          {/* i18n-ok */}Gelabber
        </Link>
        {children}
      </div>
    </main>
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
