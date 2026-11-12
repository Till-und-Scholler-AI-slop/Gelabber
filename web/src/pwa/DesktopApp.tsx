import { ChevronIcon } from "../components/Icons.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { CopyButton } from "../components/ui/CopyButton.tsx";
import { T, useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { pwa } from "../i18n/catalogs/pwa.ts";
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
  const t = useT(pwa);
  const other: DesktopPlatform = platform === "windows" ? "linux" : "windows";
  return (
    <section
      className="pwa-install desktop-app"
      aria-label={t("desktop.label")}
    >
      <div className="pwa-install-heading">
        <img src="/icons/icon-192.png" alt="" width={48} height={48} />
        <div>
          <h2>
            {t("desktop.title", { platform: DESKTOP_PLATFORM_LABEL[platform] })}
          </h2>
          <p>{t("desktop.pushToTalk")}</p>
        </div>
      </div>
      <PlatformSteps platform={platform} server={server} />
      <details className="pwa-install-disclosure desktop-app-other">
        <summary>
          {t("desktop.other", { platform: DESKTOP_PLATFORM_LABEL[other] })}
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
  const t = useT(pwa);
  return platform === "windows" ? (
    <div className="desktop-app-steps">
      <a
        className={buttonClass({ variant: "primary" })}
        href={WINDOWS_SETUP_URL}
      >
        {t("windows.download")}
      </a>
      <ol>
        <li>{t("windows.run")}</li>
        <li>
          <ServerStep server={server} />
        </li>
      </ol>
      <p className="pwa-install-note">{t("windows.updates")}</p>
    </div>
  ) : (
    <div className="desktop-app-steps">
      <h3>{t("arch.title")}</h3>
      <p>{t("arch.terminal")}</p>
      <Command text={ARCH_INSTALL_COMMAND} />
      <p>
        <T catalog={pwa} k="arch.body" tags={{ code: inlineCode }} />
      </p>
      <h3>{t("linux.title")}</h3>
      <p>
        <T
          catalog={pwa}
          k="linux.body"
          tags={{
            link: (text) => (
              <a className="living-room-link" href={LINUX_TARBALL_URL}>
                {text}
              </a>
            ),
            code: inlineCode,
          }}
        />
      </p>
      <p>
        <ServerStep server={server} />
      </p>
    </div>
  );
}

const inlineCode = (text: string) => <code>{text}</code>;

function ServerStep({ server }: { server: string }) {
  return (
    <T
      catalog={pwa}
      k="desktop.server"
      params={{ server }}
      tags={{ code: inlineCode }}
    />
  );
}

function Command({ text }: { text: string }) {
  const t = useT(common);
  return (
    <div className="desktop-app-command">
      <code>{text}</code>
      <CopyButton size="sm" onCopy={() => copyText(text)}>
        {t("copy")}
      </CopyButton>
    </div>
  );
}
