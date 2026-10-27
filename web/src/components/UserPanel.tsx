import { Link, useNavigate } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import {
  STATUS_TEXT_MAX,
  cardFormOf,
  fieldMessage,
  validateCardForm,
} from "../auth/rules.ts";
import {
  claimProfileWrite,
  updateProfile,
  useProfileWrites,
  useSession,
} from "../auth/session.ts";
import type { User } from "../auth/types.ts";
import { invokeNative, isDesktopApp } from "../voice/native/bridge.ts";
import { usePresenceStore } from "../ws/live.ts";
import type { PresenceStatus } from "../ws/protocol.ts";
import { ChevronIcon, GearIcon } from "./Icons.tsx";
import { logOut } from "./logout.ts";
import { useMenuDetails } from "./menuDetails.ts";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { Button } from "./ui/Button.tsx";
import { InlineStatus } from "./ui/InlineStatus.tsx";
import { Input } from "./ui/Input.tsx";
import { Popover } from "./ui/Popover.tsx";
import { useInlineStatus } from "./ui/useInlineStatus.ts";
import { ownPresence, panelSubtitle, PRESENCE_LABEL } from "./userPresence.ts";

export function UserPanel({ contextId }: { contextId?: string }) {
  const user = useSession((state) => state.user);
  const presence = usePresenceStore((state) =>
    user ? ownPresence(state.byServer, user.id, contextId) : "o",
  );
  const navigate = useNavigate();
  const menu = useMenuDetails();
  const profileButton = useRef<HTMLButtonElement>(null);
  const [statusOpen, setStatusOpen] = useState(false);
  if (!user) return null;

  return (
    <div className="user-dock">
      <footer className="user-panel">
        <button
          ref={profileButton}
          type="button"
          className="user-panel-profile"
          aria-haspopup="dialog"
          aria-expanded={statusOpen}
          onClick={() => setStatusOpen((open) => !open)}
        >
          <PresenceAvatar
            name={user.name}
            url={user.avatar_url}
            status={presence}
            size="md"
            decoration={user.avatar_decoration}
            accent={user.accent_color}
          />
          <span>
            <strong>{user.name}</strong>
            <small>{panelSubtitle(presence, user.status_text)}</small>
          </span>
        </button>
        <Popover
          open={statusOpen}
          onClose={() => setStatusOpen(false)}
          anchor={profileButton}
          label="Dein Status"
          placement="top-start"
          className="user-status-popover"
        >
          <StatusCard
            user={user}
            presence={presence}
            onDone={() => setStatusOpen(false)}
          />
        </Popover>
        <Link
          to="/settings/$area"
          params={{ area: "darstellung" }}
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
            <Link to="/settings/$area" params={{ area: "profil" }}>
              Profil bearbeiten
            </Link>
            <Link to="/settings/$area" params={{ area: "darstellung" }}>
              Einstellungen
            </Link>
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
            <button
              type="button"
              onClick={async () => {
                if (await logOut()) void navigate({ to: "/login" });
              }}
            >
              Abmelden
            </button>
          </nav>
        </details>
      </footer>
    </div>
  );
}

/** The status popover: who you are right now, and the status text to edit. */
function StatusCard({
  user,
  presence,
  onDone,
}: {
  user: User;
  presence: PresenceStatus;
  onDone: () => void;
}) {
  const [text, setText] = useState(user.status_text ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const status = useInlineStatus();
  const busy = useProfileWrites((state) => state.pending > 0);
  const changed = text.trim() !== (user.status_text ?? "");

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!changed) return;
    const code = validateCardForm({
      ...cardFormOf(user),
      status_text: text,
    }).status_text;
    if (code) {
      setError(fieldMessage("status_text", code));
      return;
    }
    // One profile write at a time, like the profile forms.
    const release = claimProfileWrite();
    if (!release) return;
    setError(null);
    setSaving(true);
    try {
      const updated = await updateProfile({ status_text: text.trim() });
      setText(updated.status_text ?? "");
      status.show({ tone: "success", message: "Gespeichert." });
    } catch {
      status.show({
        tone: "error",
        message: "Nicht gespeichert – erneut versuchen",
      });
    } finally {
      release();
      setSaving(false);
    }
  };

  return (
    <div className="user-status-card">
      <div className="user-status-card__who">
        <PresenceAvatar
          name={user.name}
          url={user.avatar_url}
          status={presence}
          size="md"
          decoration={user.avatar_decoration}
          accent={user.accent_color}
        />
        <span>
          <strong>{user.name}</strong>
          <small>{PRESENCE_LABEL[presence]}</small>
        </span>
      </div>
      <form className="user-status-card__form" onSubmit={onSubmit} noValidate>
        <label htmlFor="user-status-text" className="ui-label">
          Status
        </label>
        <div className="user-status-card__row">
          <Input
            id="user-status-text"
            value={text}
            maxLength={STATUS_TEXT_MAX}
            placeholder="Was machst du gerade?"
            invalid={Boolean(error)}
            aria-describedby={error ? "user-status-text-error" : undefined}
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
          />
          <Button
            type="submit"
            size="sm"
            disabled={!changed || (busy && !saving)}
            pending={saving}
            pendingLabel="Speichert…"
          >
            Speichern
          </Button>
        </div>
        {error ? (
          <p id="user-status-text-error" className="ui-field-error">
            {error}
          </p>
        ) : null}
        <InlineStatus status={status.status} />
      </form>
      <Link
        to="/settings/$area"
        params={{ area: "profil" }}
        className="user-status-card__link"
        onClick={onDone}
      >
        Profil bearbeiten
      </Link>
    </div>
  );
}
