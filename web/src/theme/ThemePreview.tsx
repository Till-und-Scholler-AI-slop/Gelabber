import type { CSSProperties } from "react";
import { HashIcon, SpeakerIcon, ChatIcon } from "../components/Icons.tsx";
import { useT } from "../i18n/catalog.ts";
import { themes } from "../i18n/catalogs/themes.ts";
import { themeVariables, type ThemeDefinition } from "./model.ts";
// Names in the sample, the same in every language.
const SAMPLE = { brand: "Gelabber", person: "Mara", initials: "MA" };
export function ThemePreview({
  theme,
  small = false,
}: {
  theme: ThemeDefinition;
  small?: boolean;
}) {
  const t = useT(themes);
  return (
    <div
      className={`theme-preview${small ? " theme-preview-small" : ""}`}
      style={themeVariables(theme) as CSSProperties}
      aria-hidden="true"
    >
      <aside className="theme-preview-sidebar">
        <strong>{SAMPLE.brand}</strong>
        <span>{t("preview.channels")}</span>
        <b>
          <ChatIcon size={12} /> {t("preview.overview")}
        </b>
        <span>
          <HashIcon size={12} /> {t("preview.general")}
        </span>
        <span>
          <SpeakerIcon size={12} /> {t("preview.livingRoom")}
        </span>
      </aside>
      <div className="theme-preview-chat">
        <header>
          <strong>
            <HashIcon size={12} /> {t("preview.general")}
          </strong>
          <span>{t("preview.topic")}</span>
        </header>
        <div className="theme-preview-message">
          <i>{SAMPLE.initials}</i>
          <div>
            <strong>
              {SAMPLE.person} <small>18:42</small>
            </strong>
            <p>{t("preview.question")}</p>
          </div>
        </div>
        <div className="theme-preview-message">
          <i>{t("preview.youInitials")}</i>
          <div>
            <strong>
              {t("preview.you")} <small>18:43</small>
            </strong>
            <p>{t("preview.answer")}</p>
          </div>
        </div>
        <div className="theme-preview-composer">
          {t("preview.composer")}
          <b>{t("preview.send")}</b>
        </div>
      </div>
      <footer>
        <span>
          <em /> {t("preview.connected")}
        </span>
        <b>{t("preview.micOn")}</b>
      </footer>
    </div>
  );
}
