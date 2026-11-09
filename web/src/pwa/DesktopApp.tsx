import { ChevronIcon } from "../components/Icons.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { CopyButton } from "../components/ui/CopyButton.tsx";
import { copyText } from "../servers/clipboard.ts";
import {
  ARCH_INSTALL_COMMAND,
  DESKTOP_PLATFORM_LABEL,
  LINUX_TARBALL_URL,
  WINDOWS_SETUP_URL,
  type DesktopPlatform,
} from "./desktopApp.ts";
import { useInstallation } from "./install.ts";
import "./install.css";

/** Settings → App on Windows and Linux: download and set up the desktop app. */
export function DesktopApp() {
  const platform = useInstallation((state) => state.desktop);
  if (!platform) return null;
  return (
    <DesktopAppPanel platform={platform} server={window.location.origin} />
  );
}

/** This system's steps first, the other system's behind one line. */
export function DesktopAppPanel({
  platform,
  server,
}: {
  platform: DesktopPlatform;
  /** The address the app asks for on its first start: this server's. */
  server: string;
}) {
  const other: DesktopPlatform = platform === "windows" ? "linux" : "windows";
  return (
    <section className="pwa-install desktop-app" aria-label="Desktop-App">
      <div className="pwa-install-heading">
        <img src="/icons/icon-192.png" alt="" width={48} height={48} />
        <div>
          <h2>Desktop-App für {DESKTOP_PLATFORM_LABEL[platform]}</h2>
          <p>Push-to-Talk wirkt auch, wenn Gelabber im Hintergrund ist.</p>
        </div>
      </div>
      <PlatformSteps platform={platform} server={server} />
      <details className="pwa-install-disclosure desktop-app-other">
        <summary>
          Für {DESKTOP_PLATFORM_LABEL[other]}
          <ChevronIcon size={14} />
        </summary>
        <PlatformSteps platform={other} server={server} />
      </details>
    </section>
  );
}

function PlatformSteps({
  platform,
  server,
}: {
  platform: DesktopPlatform;
  server: string;
}) {
  return platform === "windows" ? (
    <div className="desktop-app-steps">
      <a
        className={buttonClass({ variant: "primary" })}
        href={WINDOWS_SETUP_URL}
      >
        Für Windows herunterladen
      </a>
      <ol>
        <li>
          Starte die heruntergeladene Datei. Zeigt Windows „Der Computer wurde
          durch Windows geschützt“, wähle „Weitere Informationen“ und dann
          „Trotzdem ausführen“.
        </li>
        <li>
          <ServerStep server={server} />
        </li>
      </ol>
      <p className="pwa-install-note">
        Neue Versionen installiert die App danach selbst.
      </p>
    </div>
  ) : (
    <div className="desktop-app-steps">
      <h3>Arch Linux und Omarchy</h3>
      <p>Im Terminal:</p>
      <Command text={ARCH_INSTALL_COMMAND} />
      <p>
        Das richtet die Paketquelle von Gelabber ein und installiert die App.
        Updates kommen mit <code>sudo pacman -Syu</code> (oder{" "}
        <code>yay -Syu</code>) zusammen mit dem Rest des Systems.
      </p>
      <h3>Andere Distributionen</h3>
      <p>
        <a className="living-room-link" href={LINUX_TARBALL_URL}>
          gelabber-native-linux-x64.tar.gz
        </a>{" "}
        herunterladen, entpacken und darin <code>./gelabber</code> starten. Neue
        Versionen meldet die App; entpacke sie dann über die alte.
      </p>
      <p>
        <ServerStep server={server} />
      </p>
    </div>
  );
}

function ServerStep({ server }: { server: string }) {
  return (
    <>
      Beim ersten Start fragt die App nach dem Server: <code>{server}</code>
    </>
  );
}

function Command({ text }: { text: string }) {
  return (
    <div className="desktop-app-command">
      <code>{text}</code>
      <CopyButton size="sm" onCopy={() => copyText(text)}>
        Kopieren
      </CopyButton>
    </div>
  );
}
