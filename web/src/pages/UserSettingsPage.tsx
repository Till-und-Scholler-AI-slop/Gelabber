import { useState } from "react";
import { useCanGoBack, useRouter } from "@tanstack/react-router";
import { useVoice } from "../voice/session.ts";
import {
  CameraIcon,
  ChatIcon,
  MicIcon,
  ScreenIcon,
} from "../components/Icons.tsx";
import { MediaSettingsForm } from "../voice/VoiceSettings.tsx";
import { ThemesPanel } from "../theme/ThemesPanel.tsx";
import { useInterfacePreferences } from "../interface/preferences.ts";
import { NotificationPermissionStatus } from "../components/NotificationPermission.tsx";
import { InstallApp } from "../pwa/InstallApp.tsx";
import { useInstallation } from "../pwa/install.ts";
import "./settings.css";

const areas = [
  {
    id: "install",
    label: "App installieren",
    description: "Gelabber direkt von deinem Startbildschirm öffnen.",
    icon: ScreenIcon,
  },
  {
    id: "appearance",
    label: "Darstellung",
    description: "Mach Gelabber zu deinem Platz.",
    icon: ScreenIcon,
  },
  {
    id: "themes",
    label: "Themes",
    description: "Farben und Designs, die zu dir passen.",
    icon: ScreenIcon,
  },
  {
    id: "audio",
    label: "Audio",
    description: "Dein Mikrofon, deine Lautsprecher und die Lautstärke.",
    icon: MicIcon,
  },
  {
    id: "video",
    label: "Video",
    description: "Kamera, Bildschirmfreigabe und Liveübertragungen.",
    icon: CameraIcon,
  },
  {
    id: "notifications",
    label: "Benachrichtigungen",
    description: "Entscheide, wie neue Nachrichten dich erreichen.",
    icon: ChatIcon,
  },
] as const;

export function UserSettingsPage() {
  const native = useInstallation((state) => state.native);
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const goBack = () => {
    if (canGoBack) {
      router.history.back();
      return;
    }
    const voice = useVoice.getState();
    const serverId = voice.serverId ?? voice.watchServerId;
    const channelId = voice.channelId ?? voice.watchChannelId;
    if (serverId && channelId) {
      void router.navigate({
        to: "/s/$serverId/c/$channelId",
        params: { serverId, channelId },
      });
    } else {
      void router.navigate({ to: "/" });
    }
  };
  const [area, setArea] = useState<(typeof areas)[number]["id"]>("appearance");
  const [themeDirty, setThemeDirty] = useState(false);
  const preferences = useInterfacePreferences();
  const selected = areas.find((item) => item.id === area)!;
  return (
    <section className="settings-page" aria-labelledby="settings-heading">
      <button type="button" className="settings-back" onClick={goBack}>
        <span aria-hidden="true">←</span> Zurück
      </button>
      <header className="settings-page-heading">
        <h1 id="settings-heading">Einstellungen</h1>
        <p>Dein Gelabber. So, wie es dir gefällt.</p>
      </header>
      <div className="settings-layout">
        <nav className="settings-navigation" aria-label="Einstellungsbereiche">
          {areas
            .filter(({ id }) => id !== "install" || !native)
            .map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                type="button"
                aria-current={area === id ? "page" : undefined}
                aria-controls="settings-content"
                onClick={() => {
                  if (
                    id !== area &&
                    themeDirty &&
                    !window.confirm("Ungespeicherten Theme-Entwurf verwerfen?")
                  )
                    return;
                  setArea(id);
                }}
              >
                <Icon size={18} />
                <span>{label}</span>
              </button>
            ))}
        </nav>
        <section
          id="settings-content"
          className="settings-content"
          aria-labelledby="settings-area-heading"
        >
          <header className="settings-content-heading">
            <h2 id="settings-area-heading">{selected.label}</h2>
            <p>{selected.description}</p>
          </header>
          {area === "install" ? (
            <InstallApp />
          ) : area === "themes" ? (
            <ThemesPanel onDirtyChange={setThemeDirty} />
          ) : area === "appearance" ? (
            <>
              <fieldset className="settings-group">
                <legend>Dein Wohnzimmer</legend>
                <label className="settings-preference">
                  <span>
                    <strong>Kompakte Raumansicht</strong>
                    <small>Kleinere Avatare und weniger Abstand.</small>
                  </span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={preferences.compactRooms}
                    onChange={(event) =>
                      preferences.patch({ compactRooms: event.target.checked })
                    }
                  />
                </label>
                <label className="settings-preference">
                  <span>
                    <strong>Bewegung reduzieren</strong>
                    <small>
                      Berücksichtigt auch die Einstellung deines Geräts.
                    </small>
                  </span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={preferences.reducedMotion}
                    onChange={(event) =>
                      preferences.patch({ reducedMotion: event.target.checked })
                    }
                  />
                </label>
              </fieldset>
            </>
          ) : (
            <>
              <MediaSettingsForm key={area} section={area} />
              {area === "notifications" && <NotificationPermissionStatus />}
            </>
          )}
          {area !== "install" && (
            <p className="settings-save-note">
              {area === "themes"
                ? "Themes werden in deinem Account gespeichert."
                : "Änderungen werden automatisch in diesem Browser gespeichert."}
            </p>
          )}
        </section>
      </div>
    </section>
  );
}
