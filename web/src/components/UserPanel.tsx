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
import { usePresenceStore } from "../ws/live.ts";
import type { PresenceStatus } from "../ws/protocol.ts";
import { moveInAccount } from "./accountKeys.ts";
import { GearIcon } from "./Icons.tsx";
import { logOut } from "./logout.ts";
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
import "./shell.css";

export function UserPanel({ contextId }: { contextId?: string }) {
  const t = useT(shell);
  const user = useSession((state) => state.user);
  const presence = usePresenceStore((state) =>
    user ? ownPresence(state.byServer, user.id, contextId) : "o",
  );
  const profileButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
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
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <span className="user-panel-avatar">
            <PresenceAvatar
              name={user.name}
              url={user.avatar_url}
              status={presence}
              size="md"
              decoration={user.avatar_decoration}
              accent={user.accent_color}
            />
            {unread ? <span className="unread-dot" aria-hidden="true" /> : null}
          </span>
          <span>
            <strong>{user.name}</strong>
            <small>{panelSubtitle(presence, user.status_text)}</small>
            {unread ? (
              <span className="sr-only">
                {", "}
                {t("userPanel.feedbackUnread")}
              </span>
            ) : null}
          </span>
        </button>
        <Popover
          open={open}
          onClose={() => setOpen(false)}
          anchor={profileButton}
          label={t("userPanel.accountLabel")}
          placement="top-start"
          className="user-account-popover"
        >
          <div className="user-account" onKeyDown={moveInAccount}>
            <StatusCard user={user} presence={presence} />
            <AccountMenu
              feedback={feedback}
              unread={unread}
              onDone={() => setOpen(false)}
              onFeedback={() => {
                // The popover closes first, and the avatar holds the focus,
                // so the dialog hands it back there when it closes.
                setOpen(false);
                profileButton.current?.focus({ preventScroll: true });
                openFeedbackDialog();
              }}
            />
          </div>
        </Popover>
        {/* Always Settings, in a call too: the call's own settings are in
            the voice room and in Settings → Audio (#570). */}
        <Link
          to="/settings/$area"
          params={{ area: "darstellung" }}
          className="shell-icon-button"
          aria-label={t("userPanel.settings")}
          title={t("userPanel.settings")}
        >
          <GearIcon size={21} />
        </Link>
      </footer>
    </div>
  );
}

/** The account's entries under the status card: profile, feedback,
 * operator, then signing out after a line. Room for the presence modes of
 * #590 above them. */
export function AccountMenu({
  feedback,
  unread,
  onDone,
  onFeedback,
}: {
  feedback: boolean;
  unread: boolean;
  onDone: () => void;
  onFeedback: () => void;
}) {
  const t = useT(shell);
  const operator = useSession((state) => state.operator);
  const navigate = useNavigate();
  return (
    <div
      role="menu"
      aria-label={t("userPanel.menuLabel")}
      className="user-account-menu"
    >
      <hr className="shell-menu-separator" />
      <Link
        to="/settings/$area"
        params={{ area: "profil" }}
        role="menuitem"
        className="shell-menu-item"
        onClick={onDone}
      >
        {t("userPanel.editProfile")}
      </Link>
      {feedback ? (
        <button
          type="button"
          role="menuitem"
          className="shell-menu-item"
          onClick={onFeedback}
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
          role="menuitem"
          className="shell-menu-item"
          onClick={onDone}
        >
          {t("userPanel.operator")}
        </Link>
      ) : null}
      <hr className="shell-menu-separator" />
      <button
        type="button"
        role="menuitem"
        className="shell-menu-item"
        onClick={async () => {
          onDone();
          if (await logOut()) void navigate({ to: "/login" });
        }}
      >
        {t("userPanel.signOut")}
      </button>
    </div>
  );
}

/** The popover's top: who you are right now, and the status text to edit. */
function StatusCard({
  user,
  presence,
}: {
  user: User;
  presence: PresenceStatus;
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
    </div>
  );
}
