// The body of a personal settings area. Loaded on its own, after the
// settings header and navigation (see UserSettingsPage).

import { MessageAlertSettings } from "../components/MessageAlertSettings.tsx";
import { NotificationPermissionStatus } from "../components/NotificationPermission.tsx";
import { NotificationPrivacySetting } from "../components/NotificationPrivacy.tsx";
import { InstallApp } from "../pwa/InstallApp.tsx";
import { ThemesPanel } from "../theme/ThemesPanel.tsx";
import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";
import { AppearanceSettings } from "./AppearanceSettings.tsx";
import { ProfileArea, ProfileCardArea, SecurityArea } from "./ProfilePage.tsx";
import type { SettingsArea } from "./settingsAreas.ts";

export function SettingsAreaContent({ area }: { area: SettingsArea }) {
  switch (area) {
    case "profil":
      return <ProfileArea />;
    case "profilkarte":
      return <ProfileCardArea />;
    case "konto":
      return <SecurityArea />;
    case "darstellung":
      return <AppearanceSettings />;
    case "themes":
      return <ThemesPanel />;
    case "audio":
    case "video":
      return <MediaSettingsForm key={area} section={area} />;
    case "benachrichtigungen":
      return (
        <>
          <MessageAlertSettings />
          <NotificationPrivacySetting />
          <NotificationPermissionStatus />
        </>
      );
    case "app":
      return <InstallApp />;
  }
}
