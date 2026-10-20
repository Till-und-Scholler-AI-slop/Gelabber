import { Link, useNavigate } from "@tanstack/react-router";

import { logout, useSession } from "../auth/session.ts";
import { invokeNative, isDesktopApp } from "../voice/native/bridge.ts";
import { leaveVoice, stopWatching } from "../voice/session.ts";
import { usePresenceStore } from "../ws/live.ts";
import { Avatar } from "./Avatar.tsx";
import { AvatarFrame } from "./AvatarFrame.tsx";
import { ChevronIcon, GearIcon } from "./Icons.tsx";
import { useMenuDetails } from "./menuDetails.ts";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { VoiceSessionControls } from "./VoiceSessionControls.tsx";

export function UserPanel({ contextId }: { contextId?: string }) {
  const user = useSession((state) => state.user);
  const status = usePresenceStore((state) =>
    user && contextId ? state.byServer[contextId]?.[user.id] : undefined,
  );
  const navigate = useNavigate();
  const menu = useMenuDetails();
  if (!user) return null;

  function onLogout() {
    leaveVoice();
    stopWatching();
    void logout();
    void navigate({ to: "/login" });
  }

  return (
    <div className="user-dock">
      <VoiceSessionControls variant="card" />
      <footer className="user-panel">
        <Link to="/profile" className="user-panel-profile" title="Dein Profil">
          {status ? (
            <PresenceAvatar
              name={user.name}
              url={user.avatar_url}
              status={status}
              size="md"
              decoration={user.avatar_decoration}
              accent={user.accent_color}
            />
          ) : (
            <AvatarFrame
              decoration={user.avatar_decoration}
              accent={user.accent_color}
            >
              <Avatar name={user.name} url={user.avatar_url} size="md" />
            </AvatarFrame>
          )}
          <span>
            <strong>{user.name}</strong>
            <small>
              {user.status_text
                ? user.status_text
                : status === "o"
                  ? "Online"
                  : status === "i"
                    ? "Abwesend"
                    : status === "x"
                      ? "Offline"
                      : "Dein Profil"}
            </small>
          </span>
        </Link>
        <Link
          to="/settings"
          className="shell-icon-button"
          aria-label="Einstellungen"
          title="Einstellungen"
        >
          <GearIcon size={21} />
        </Link>
        <details ref={menu} className="user-panel-menu">
          <summary className="shell-icon-button" aria-label="Benutzermenü">
            <ChevronIcon size={18} />
          </summary>
          <nav aria-label="Benutzerkonto">
            <Link to="/profile">Dein Profil</Link>
            <Link to="/settings">Einstellungen</Link>
            {isDesktopApp() ? (
              <button
                type="button"
                title="Strg+Umschalt+S"
                onClick={() => {
                  // Older desktop builds lack the command; nothing to do then.
                  invokeNative("open_setup").catch(() => undefined);
                }}
              >
                Server wechseln …
              </button>
            ) : null}
            <button type="button" onClick={onLogout}>
              Abmelden
            </button>
          </nav>
        </details>
      </footer>
    </div>
  );
}
