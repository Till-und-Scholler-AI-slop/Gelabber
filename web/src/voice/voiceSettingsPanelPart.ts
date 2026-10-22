// The forms and diagnostics of the Voice & Video dialog load with its first
// opening (see VoiceSettingsDialog).

import { lazyPart } from "../components/lazyPart.tsx";

export const VoiceSettingsPanelPart = lazyPart(
  () =>
    import("./VoiceSettingsPanel.tsx").then(
      (module) => module.VoiceSettingsPanel,
    ),
  "Die Einstellungen konnten nicht geladen werden.",
);
