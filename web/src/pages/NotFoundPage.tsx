import { Link } from "@tanstack/react-router";

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
    <section className="lr-auth-card mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">
        Seite nicht gefunden
      </h1>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        Diese Adresse gibt es in Gelabber nicht.
      </p>
      <Link
        to="/"
        className="mt-6 inline-block font-medium text-neutral-900 underline underline-offset-2 dark:text-neutral-100"
      >
        Zur Startseite
      </Link>
    </section>
  );
}
