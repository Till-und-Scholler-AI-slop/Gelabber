import { useId, useRef, useState } from "react";

import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";
import { VoiceDiagnostics } from "../voice/VoiceDiagnostics.tsx";
import {
  useMediaSettings,
  type MediaDialogSection,
} from "../voice/settings.ts";
import { GhostButton, Modal } from "./Modal.tsx";
import { nextChipIndex } from "./ui/chipKeys.ts";

const TABS: readonly { id: MediaDialogSection; label: string }[] = [
  { id: "audio", label: "Audio" },
  { id: "video", label: "Video & Stream" },
  { id: "diagnostics", label: "Diagnose" },
];

export function VoiceSettingsDialog() {
  const open = useMediaSettings((s) => s.dialogOpen);
  const section = useMediaSettings((s) => s.dialogSection);
  const close = useMediaSettings((s) => s.closeDialog);
  return (
    <Modal
      open={open}
      onClose={close}
      title="Voice & Video"
      wide
      footer={<GhostButton onClick={close}>Fertig</GhostButton>}
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
  const [tab, setTab] = useState(initial);
  const ids = useId();
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  return (
    <div className="voice-settings-tabs">
      <div
        role="tablist"
        aria-label="Bereiche"
        className="voice-settings-tablist"
      >
        {TABS.map((entry, index) => (
          <button
            key={entry.id}
            ref={(element) => {
              tabs.current[index] = element;
            }}
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
            {entry.label}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id={`${ids}-panel`}
        aria-labelledby={`${ids}-${tab}-tab`}
        className="voice-settings-panel"
      >
        {tab === "diagnostics" ? (
          <VoiceDiagnostics />
        ) : (
          <MediaSettingsForm section={tab} />
        )}
      </div>
    </div>
  );
}
