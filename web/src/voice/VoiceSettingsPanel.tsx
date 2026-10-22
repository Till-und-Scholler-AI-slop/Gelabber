// The body of one tab of the Voice & Video dialog. Loaded on its own, once
// the dialog first opens (see VoiceSettingsDialog).

import type { MediaDialogSection } from "./settings.ts";
import { VoiceDiagnostics } from "./VoiceDiagnostics.tsx";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

export function VoiceSettingsPanel({ tab }: { tab: MediaDialogSection }) {
  return tab === "diagnostics" ? (
    <VoiceDiagnostics />
  ) : (
    <MediaSettingsForm section={tab} />
  );
}
