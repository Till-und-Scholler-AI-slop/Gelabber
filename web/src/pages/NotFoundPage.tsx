import { Link } from "@tanstack/react-router";

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
  return (
    <section className="lr-auth-card mx-auto w-full max-w-sm">
      <h1>Seite nicht gefunden</h1>
      <p className="auth-lead">Diese Adresse gibt es in Gelabber nicht.</p>
      <p className="auth-footer">
        <Link to="/" className="auth-link">
          Zur Startseite
        </Link>
      </p>
    </section>
  );
}
