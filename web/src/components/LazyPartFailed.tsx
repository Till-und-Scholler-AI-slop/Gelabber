// What a lazy part shows when its chunk cannot be loaded (see lazyPart).

import { useVoice } from "../voice/session.ts";
import { Button } from "./ui/Button.tsx";

export function LazyPartFailed({ failure }: { failure: string }) {
  const inCall = useVoice((s) => s.status === "joined");
  return <LazyPartFailedView failure={failure} inCall={inCall} />;
}

export function LazyPartFailedView({
  failure,
  inCall,
}: {
  failure: string;
  inCall: boolean;
}) {
  return (
    <div role="alert" className="ui-load-error">
      <p>
        {failure}{" "}
        {inCall
          ? "Vermutlich gibt es eine neue Version. Lade die Seite nach dem Anruf neu."
          : "Vermutlich gibt es eine neue Version."}
      </p>
      {inCall ? null : (
        <Button size="sm" onClick={() => window.location.reload()}>
          Seite neu laden
        </Button>
      )}
    </div>
  );
}
