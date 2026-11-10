import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { useVoice } from "../voice/session.ts";
import "./auth.css";

/** What a page shows when it throws: the router's default error component,
 * and the boundary around the page in the app shell. */
export function RouteErrorPage() {
  const inCall = useVoice((s) => s.status === "joined");
  return <RouteErrorView inCall={inCall} />;
}

export function RouteErrorView({ inCall }: { inCall: boolean }) {
  const t = useT(auth);
  return (
    <div className="px-6 py-10">
      <section role="alert" className="lr-auth-card mx-auto w-full max-w-sm">
        <h1>{t("routeError.title")}</h1>
        <p className="auth-lead">
          {inCall ? t("routeError.bodyInCall") : t("routeError.body")}
        </p>
        <div className="auth-actions">
          <Button variant="primary" onClick={() => window.location.reload()}>
            {t("routeError.reload")}
          </Button>
        </div>
      </section>
    </div>
  );
}
