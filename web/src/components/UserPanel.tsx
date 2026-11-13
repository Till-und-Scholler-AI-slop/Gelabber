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
import { QuickCallSettings } from "../voice/QuickCallSettings.tsx";
import { useVoice } from "../voice/session.ts";
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
import { ownPresence, panelSubtitle } from "./userPresence.ts";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { common } from "../i18n/catalogs/common.ts";
import { DEFAULT_OPERATOR_PAGE } from "../operator/areas.ts";
import { openFeedbackDialog } from "../feedback/dialogStore.ts";
import { useFeedbackStatus } from "../feedback/queries.ts";
import "../feedback/feedback.css";

export function UserPanel({ contextId }: { contextId?: string }) {
  const t = useT(shell);
  const user = useSession((state) => state.user);
  const operator = useSession((state) => state.operator);
  const presence = usePresenceStore((state) =>
    user ? ownPresence(state.byServer, user.id, contextId) : "o",
  );
  const navigate = useNavigate();
  const menu = useMenuDetails();
  const profileButton = useRef<HTMLButtonElement>(null);
  const [statusOpen, setStatusOpen] = useState(false);
  const inCall = useVoice((state) => state.status === "joined");
  const feedbackStatus = useFeedbackStatus().data;
  const feedback = feedbackStatus?.enabled === true;
  const unread = feedback && feedbackStatus.unread > 0;
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
          label={t("userPanel.statusLabel")}
          placement="top-start"
          className="user-status-popover"
        >
          <StatusCard
            user={user}
            presence={presence}
            onDone={() => setStatusOpen(false)}
          />
        </Popover>
        {inCall ? (
          <CallSettingsGear />
        ) : (
          <Link
            to="/settings/$area"
            params={{ area: "darstellung" }}
            className="shell-icon-button"
            aria-label={t("userPanel.settings")}
            title={t("userPanel.settings")}
          >
            <GearIcon size={21} />
          </Link>
        )}
        <details ref={menu} className="user-panel-menu">
          <summary
            className="shell-icon-button"
            aria-label={t(
              unread ? "userPanel.menuUnreadLabel" : "userPanel.menuLabel",
            )}
          >
            <ChevronIcon size={18} />
            {unread ? <span className="unread-dot" aria-hidden="true" /> : null}
          </summary>
          <nav aria-label={t("userPanel.accountLabel")}>
            <Link to="/settings/$area" params={{ area: "profil" }}>
              {t("userPanel.editProfile")}
            </Link>
            <Link to="/settings/$area" params={{ area: "darstellung" }}>
              {t("userPanel.settings")}
            </Link>
            {feedback ? (
              <button
                type="button"
                className="user-panel-feedback"
                onClick={(event) => {
                  // The menu closes first, and its chevron holds focus, so
                  // the dialog hands focus back there when it closes.
                  const details = event.currentTarget.closest("details");
                  details?.removeAttribute("open");
                  details
                    ?.querySelector<HTMLElement>(":scope > summary")
                    ?.focus();
                  openFeedbackDialog();
                }}
              >
                {t("userPanel.feedback")}
                {unread ? (
                  <>
                    <span className="sr-only">
                      {", "}
                      {t("userPanel.feedbackUnread")}
                    </span>
                    <span className="unread-dot" aria-hidden="true" />
                  </>
                ) : null}
              </button>
            ) : null}
            {operator ? (
              <Link
                to="/betrieb/$page"
                params={{ page: DEFAULT_OPERATOR_PAGE }}
              >
                {t("userPanel.operator")}
              </Link>
            ) : null}
            <button
              type="button"
              onClick={async () => {
                if (await logOut()) void navigate({ to: "/login" });
              }}
            >
              {t("userPanel.signOut")}
            </button>
          </nav>
        </details>
      </footer>
    </div>
  );
}

/** The gear during a call: the call's quick settings, as the call bar's
 * gear opened them. The app's settings stay in the chevron menu. */
function CallSettingsGear() {
  const t = useT(shell);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="shell-icon-button"
        aria-label={t("userPanel.callSettings")}
        title={t("userPanel.callSettings")}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <GearIcon size={21} />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("userPanel.callSettings")}
        placement="top-start"
        className="voice-quick-settings"
      >
        <QuickCallSettings onMore={() => setOpen(false)} />
      </Popover>
    </>
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
  const t = useT(shell);
  const tc = useT(common);
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
      status.show({ tone: "success", message: tc("saved") });
    } catch {
      status.show({
        tone: "error",
        message: tc("notSaved"),
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
          <small>{t(`presence.${presence}`)}</small>
        </span>
      </div>
      <form className="user-status-card__form" onSubmit={onSubmit} noValidate>
        <label htmlFor="user-status-text" className="ui-label">
          {t("userPanel.status")}
        </label>
        <div className="user-status-card__row">
          <Input
            id="user-status-text"
            value={text}
            maxLength={STATUS_TEXT_MAX}
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
            pendingLabel={t("userPanel.saving")}
          >
            {tc("save")}
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
        {t("userPanel.editProfile")}
      </Link>
    </div>
  );
}
