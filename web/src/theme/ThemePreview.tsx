import type { CSSProperties } from "react";
import { HashIcon, SpeakerIcon, ChatIcon } from "../components/Icons.tsx";
import { themeVariables, type ThemeDefinition } from "./model.ts";
export function ThemePreview({
  theme,
  small = false,
}: {
  theme: ThemeDefinition;
  small?: boolean;
}) {
  return (
    <div
      className={`theme-preview${small ? " theme-preview-small" : ""}`}
      style={themeVariables(theme) as CSSProperties}
      aria-hidden="true"
    >
      <aside className="theme-preview-sidebar">
        <strong>Gelabber</strong>
        <span>DEIN WOHNZIMMER</span>
        <b>
          <ChatIcon size={12} /> Übersicht
        </b>
        <span>
          <HashIcon size={12} /> allgemein
        </span>
        <span>
          <SpeakerIcon size={12} /> Wohnzimmer
        </span>
      </aside>
      <div className="theme-preview-chat">
        <header>
          <strong>
            <HashIcon size={12} /> allgemein
          </strong>
          <span>Ein guter Platz zum Ankommen.</span>
        </header>
        <div className="theme-preview-message">
          <i>MA</i>
          <div>
            <strong>
              Mara <small>18:42</small>
            </strong>
            <p>Wer ist heute Abend dabei?</p>
          </div>
        </div>
        <div className="theme-preview-message">
          <i>DU</i>
          <div>
            <strong>
              Du <small>18:43</small>
            </strong>
            <p>Ich bin im Wohnzimmer. Komm dazu!</p>
          </div>
        </div>
        <div className="theme-preview-composer">
          Nachricht schreiben …<b>Senden</b>
        </div>
      </div>
      <footer>
        <span>
          <em /> Verbunden · Wohnzimmer
        </span>
        <b>Mikrofon an</b>
      </footer>
    </div>
  );
}
