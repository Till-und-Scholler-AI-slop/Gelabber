// Notification level and mute of a server, a channel or a DM (issue #242),
// opened by the bell in its header or from its row menu (RowMenu.tsx),
// which has the quick choices. Every choice is saved at once.
import { useState } from "react";

import { ApiError } from "../api/client.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { notifications } from "../i18n/catalogs/notifications.ts";
import { BellIcon, BellOffIcon } from "./Icons.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { Button } from "./ui/Button.tsx";
import { RadioCard } from "./ui/RadioCard.tsx";
import "./notificationSettings.css";
import {
  LEVELS,
  MUTES,
  levelName,
  mutedLine,
  useNotificationActions,
} from "./notificationActions.ts";
import {
  openNotificationSettings,
  useNotificationTarget,
  type NotificationTarget,
} from "./notificationTarget.ts";

export function NotificationSettingsHost() {
  const target = useNotificationTarget((state) => state.target);
  const close = () => useNotificationTarget.setState({ target: null });
  const t = useT(notifications);
  return (
    <Modal
      open={Boolean(target)}
      onClose={close}
      title={
        target ? t("dialog.titleFor", { name: target.name }) : t("dialog.title")
      }
    >
      {target ? (
        <NotificationSettingsForm
          key={`${target.scope}:${target.id}`}
          target={target}
          onDone={close}
        />
      ) : null}
    </Modal>
  );
}

function NotificationSettingsForm({
  target,
  onDone,
}: {
  target: NotificationTarget;
  onDone: () => void;
}) {
  const t = useT(notifications);
  const tc = useT(common);
  const actions = useNotificationActions(target);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async (next: Promise<unknown>) => {
    setSaving(true);
    setError(null);
    try {
      await next;
    } catch (cause) {
      setError(
        cause instanceof ApiError && cause.status === 404
          ? t("dialog.gone")
          : t("dialog.notSaved"),
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="notify-settings flex flex-col gap-5">
      <section aria-labelledby="notify-mute" className="flex flex-col gap-2">
        <h3 id="notify-mute" className="text-sm font-semibold">
          {t("dialog.mute")}
        </h3>
        {actions.muted ? (
          <div className="notify-muted">
            <span className="inline-flex items-center gap-2">
              <BellOffIcon size={16} />
              {mutedLine(actions.own.muted_until)}
            </span>
            <Button
              size="sm"
              disabled={saving}
              onClick={() => void save(actions.unmute())}
            >
              {t("dialog.unmute")}
            </Button>
          </div>
        ) : (
          // One row of chips: how long, in short; named in full.
          <div
            role="group"
            aria-labelledby="notify-mute"
            className="notify-mute-row"
          >
            {MUTES.map((mute) => (
              <button
                key={mute.label}
                type="button"
                disabled={saving}
                title={mute.label}
                aria-label={mute.label}
                className="ui-chip"
                onClick={() => void save(actions.mute(mute.minutes))}
              >
                {mute.short}
              </button>
            ))}
          </div>
        )}
        <p className="ui-hint">
          {target.dm ? t("dialog.muteHint.dm") : t("dialog.muteHint.channel")}
          {actions.serverMuted ? ` ${t("dialog.serverMuted")}` : ""}
        </p>
      </section>
      {target.dm ? null : (
        <fieldset className="flex flex-col gap-2" disabled={saving}>
          <legend className="mb-1 text-sm font-semibold">
            {t("dialog.levelLegend")}
          </legend>
          {target.serverId ? (
            <LevelOption
              name={target.id}
              checked={actions.isLevel(null)}
              label={t("level.inherit", {
                level: levelName(actions.inherited),
              })}
              onChange={() => void save(actions.setLevel(null))}
            />
          ) : null}
          {LEVELS.map((row) => (
            <LevelOption
              key={row.level}
              name={target.id}
              checked={actions.isLevel(row.level)}
              label={row.label}
              hint={row.hint}
              onChange={() => void save(actions.setLevel(row.level))}
            />
          ))}
        </fieldset>
      )}
      {error ? (
        <p role="alert" className="ui-field-error">
          {error}
        </p>
      ) : null}
      <div className="notify-footer">
        <MarkRead target={target} onDone={onDone} />
        <GhostButton onClick={onDone} initialFocus>
          {tc("done")}
        </GhostButton>
      </div>
    </div>
  );
}

/** "Als gelesen markieren" (#245): the conversation or the whole server,
 * then the dialog closes. */
function MarkRead({
  target,
  onDone,
}: {
  target: NotificationTarget;
  onDone: () => void;
}) {
  const t = useT(notifications);
  const { markRead } = useNotificationActions(target);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  return (
    <div className="flex flex-col gap-1">
      <Button
        size="sm"
        className="self-start"
        disabled={busy}
        onClick={() => {
          setBusy(true);
          setFailed(false);
          markRead()
            .then(onDone)
            .catch(() => setFailed(true))
            .finally(() => setBusy(false));
        }}
      >
        {t("dialog.markRead")}
      </Button>
      {failed ? (
        <p role="alert" className="ui-field-error">
          {t("dialog.notSaved")}
        </p>
      ) : null}
    </div>
  );
}

function LevelOption({
  name,
  checked,
  label,
  hint,
  onChange,
}: {
  name: string;
  checked: boolean;
  label: string;
  hint?: string;
  onChange: () => void;
}) {
  return (
    <RadioCard
      name={`notify-level-${name}`}
      checked={checked}
      onChange={onChange}
      label={label}
      description={hint}
    />
  );
}

/** The bell in a header: shows a mute, opens the dialog. */
export function NotificationBell({
  target,
  muted,
}: {
  target: NotificationTarget;
  muted: boolean;
}) {
  const t = useT(notifications);
  return (
    <button
      type="button"
      className="lr-header-icon notify-bell"
      aria-label={t(muted ? "bell.labelMuted" : "bell.label", {
        name: target.name,
      })}
      title={muted ? t("bell.titleMuted") : t("bell.title")}
      onClick={() => openNotificationSettings(target)}
    >
      {muted ? <BellOffIcon size={16} /> : <BellIcon size={16} />}
    </button>
  );
}
