// The diagnostics at the end of Settings → Audio (#570). The call bar's
// connection popover and "Alle Details" link here with the section's hash.

import { useLocation } from "@tanstack/react-router";
import { useEffect, useId, useRef } from "react";

import { useT } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import { DIAGNOSTICS_SECTION } from "./mediaSettingsLink.ts";
import { VoiceDiagnostics } from "./VoiceDiagnostics.tsx";

export function DiagnosticsSection() {
  const t = useT(voiceSettings);
  const id = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const wanted = useLocation({ select: (l) => l.hash === DIAGNOSTICS_SECTION });

  // Asked for by the link: in view, with the focus on its heading. After
  // the page's own move to its top and its heading (UserSettingsPage).
  useEffect(() => {
    if (!wanted) return;
    const frame = requestAnimationFrame(() => {
      heading.current?.scrollIntoView({ block: "start" });
      heading.current?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [wanted]);

  return (
    <section
      id={DIAGNOSTICS_SECTION}
      className="media-group settings-diagnostics"
      aria-labelledby={id}
    >
      <header className="media-group-head">
        <h3 id={id} ref={heading} tabIndex={-1}>
          {t("diagnostics.title")}
        </h3>
      </header>
      <VoiceDiagnostics />
    </section>
  );
}
