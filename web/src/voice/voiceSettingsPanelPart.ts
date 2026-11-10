// The forms and diagnostics of the Audio & Video dialog load with its first
// opening (see VoiceSettingsDialog).

import { lazyPart } from "../components/lazyPart.tsx";
import { translate } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";

export const VoiceSettingsPanelPart = lazyPart(
  () =>
    import("./VoiceSettingsPanel.tsx").then(
      (module) => module.VoiceSettingsPanel,
    ),
  translate(voiceSettings, "panel.loadFailed"),
);
