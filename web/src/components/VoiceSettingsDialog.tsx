import { useId, useRef, useState } from "react";

import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import {
  useMediaSettings,
  type MediaDialogSection,
} from "../voice/settings.ts";
import { VoiceSettingsPanelPart } from "../voice/voiceSettingsPanelPart.ts";
import { GhostButton, Modal } from "./Modal.tsx";
import { nextChipIndex } from "./ui/chipKeys.ts";
import { SkeletonRows } from "./ui/Skeleton.tsx";

const TABS = [
  { id: "audio", label: "tabs.audio" },
  { id: "video", label: "tabs.video" },
  { id: "diagnostics", label: "tabs.diagnostics" },
] as const satisfies readonly {
  id: MediaDialogSection;
  label: keyof typeof voiceSettings.de;
}[];

export function VoiceSettingsDialog() {
  const t = useT(voiceSettings);
  const tc = useT(common);
  const open = useMediaSettings((s) => s.dialogOpen);
  const section = useMediaSettings((s) => s.dialogSection);
  const close = useMediaSettings((s) => s.closeDialog);
  return (
    <Modal
      open={open}
      onClose={close}
      title={t("dialog.title")}
      large
      footer={<GhostButton onClick={close}>{tc("done")}</GhostButton>}
    >
      {/* Opened again on another tab, it starts there. */}
      <VoiceSettingsTabs
        key={section ?? "audio"}
        initial={section ?? "audio"}
      />
    </Modal>
  );
}

/** Audio, video and the diagnostics, one at a time instead of one long
 * scroll. */
export function VoiceSettingsTabs({
  initial,
}: {
  initial: MediaDialogSection;
}) {
  const t = useT(voiceSettings);
  const [tab, setTab] = useState<MediaDialogSection>(initial);
  const ids = useId();
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  return (
    <div className="voice-settings-tabs">
      <div
        role="tablist"
        aria-label={t("tabs.label")}
        className="voice-settings-tablist"
      >
        {TABS.map((entry, index) => (
          <button
            key={entry.id}
            ref={(element) => {
              tabs.current[index] = element;
              // The dialog opens on the chosen tab, not the first one
              // (see dialogFocus.ts).
              element?.toggleAttribute("autofocus", tab === entry.id);
            }}
            autoFocus={tab === entry.id}
            type="button"
            role="tab"
            id={`${ids}-${entry.id}-tab`}
            aria-selected={tab === entry.id}
            aria-controls={`${ids}-panel`}
            tabIndex={tab === entry.id ? 0 : -1}
            onClick={() => setTab(entry.id)}
            onKeyDown={(event) => {
              const next = nextChipIndex(
                event.key,
                index,
                TABS.map(() => false),
              );
              if (next === null) return;
              event.preventDefault();
              setTab(TABS[next].id);
              tabs.current[next]?.focus();
            }}
          >
            {t(entry.label)}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id={`${ids}-panel`}
        aria-labelledby={`${ids}-${tab}-tab`}
        className="voice-settings-panel"
      >
        <VoiceSettingsPanelPart.Component
          tab={tab}
          fallback={<SkeletonRows count={4} />}
        />
      </div>
    </div>
  );
}
