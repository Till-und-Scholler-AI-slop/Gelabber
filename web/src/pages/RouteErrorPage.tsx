import { Button } from "../components/ui/Button.tsx";
import { useVoice } from "../voice/session.ts";
import "./auth.css";

/** What a page shows when it throws: the router's default error component,
 * and the boundary around the page in the app shell. */
export function RouteErrorPage() {
  const inCall = useVoice((s) => s.status === "joined");
  return <RouteErrorView inCall={inCall} />;
}

export function RouteErrorView({ inCall }: { inCall: boolean }) {
  return (
    <div className="px-6 py-10">
      <section role="alert" className="lr-auth-card mx-auto w-full max-w-sm">
        <h1>Etwas ist schiefgelaufen</h1>
        <p className="auth-lead">
          {inCall
            ? "Diese Seite konnte nicht angezeigt werden. Dein Anruf läuft weiter; neu laden beendet ihn."
            : "Diese Seite konnte nicht angezeigt werden."}
        </p>
        <div className="auth-actions">
          <Button variant="primary" onClick={() => window.location.reload()}>
            Seite neu laden
          </Button>
        </div>
      </section>
    </div>
  );
}
