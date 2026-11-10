import { Link } from "@tanstack/react-router";

import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";

import "./auth.css";

/** Any path the app does not know, as a whole page (no layout around it). */
export function NotFoundPage() {
  return (
    <main className="mx-auto max-w-3xl px-6 py-10">
      <NotFoundCard />
    </main>
  );
}

/** The same notice inside a layout whose `<Outlet />` already sits in a
 * `<main>`. */
export function NotFoundCard() {
  const t = useT(auth);
  return (
    <section className="lr-auth-card mx-auto w-full max-w-sm">
      <h1>{t("notFound.title")}</h1>
      <p className="auth-lead">{t("notFound.body")}</p>
      <p className="auth-footer">
        <Link to="/" className="auth-link">
          {t("notFound.home")}
        </Link>
      </p>
    </section>
  );
}
