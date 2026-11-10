// What a lazy part shows when its chunk cannot be loaded (see lazyPart).

import { useVoice } from "../voice/session.ts";
import { Button } from "./ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

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
  const t = useT(shell);
  return (
    <div role="alert" className="ui-load-error">
      <p>
        {failure}{" "}
        {t(inCall ? "lazyPart.newVersionInCall" : "lazyPart.newVersion")}
      </p>
      {inCall ? null : (
        <Button size="sm" onClick={() => window.location.reload()}>
          {t("lazyPart.reload")}
        </Button>
      )}
    </div>
  );
}
