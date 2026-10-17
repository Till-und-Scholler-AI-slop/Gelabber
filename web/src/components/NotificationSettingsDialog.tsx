// Notification level and mute of a server, a channel or a DM (issue #242),
// opened by a right-click (a long press on phones) on its entry or by the
// bell in its header. Every choice is saved at once.
import { useState } from "react";

import { ApiError } from "../api/client.ts";
import {
  settingOf,
  useChangeNotificationSetting,
  useMuteClock,
  useNotificationSettings,
  type SettingChange,
} from "../messages/notificationSettings.ts";
import { isMuted, type NotifyLevel } from "../messages/notifyLevel.ts";
import { useMarkManyRead } from "../messages/readState.ts";
import { BellIcon, BellOffIcon } from "./Icons.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import {
  minutesUntil,
  openNotificationSettings,
  useNotificationTarget,
  type NotificationTarget,
} from "./notificationTarget.ts";

const MUTES: { label: string; minutes?: number }[] = [
  { label: "15 Minuten", minutes: 15 },
  { label: "1 Stunde", minutes: 60 },
  { label: "8 Stunden", minutes: 8 * 60 },
  { label: "24 Stunden", minutes: 24 * 60 },
  { label: "Bis ich es wieder einschalte" },
];

const LEVELS: { level: NotifyLevel; label: string; hint: string }[] = [
  { level: "all", label: "Alle Nachrichten", hint: "Jede neue Nachricht" },
  {
    level: "mentions",
    label: "Nur @Erwähnungen",
    hint: "Wenn dich jemand erwähnt oder @everyone/@here schreibt",
  },
  {
    level: "none",
    label: "Nichts",
    hint: "Keine Benachrichtigungen, kein Zähler",
  },
];

const levelName = (level: NotifyLevel) =>
  LEVELS.find((row) => row.level === level)!.label;

export function NotificationSettingsHost() {
  const target = useNotificationTarget((state) => state.target);
  const close = () => useNotificationTarget.setState({ target: null });
  return (
    <Modal
      open={Boolean(target)}
      onClose={close}
      title={
        target ? `Benachrichtigungen: ${target.name}` : "Benachrichtigungen"
      }
    >
      {target?.allDms ? (
        <div className="flex flex-col gap-5">
          <MarkRead target={target} onDone={close} />
          <div className="flex justify-end">
            <GhostButton onClick={close} initialFocus>
              Fertig
            </GhostButton>
          </div>
        </div>
      ) : target ? (
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
  const { data: settings } = useNotificationSettings();
  const change = useChangeNotificationSetting();
  const now = useMuteClock();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const own = settingOf(settings, target.scope, target.id);
  const server = target.serverId
    ? settingOf(settings, "server", target.serverId)
    : null;
  const muted = isMuted(own, now);
  const serverMuted = server ? isMuted(server, now) : false;
  const inherited: NotifyLevel = server?.level ?? "all";

  const save = async (next: SettingChange) => {
    setSaving(true);
    setError(null);
    try {
      await change(target.scope, target.id, next);
    } catch (cause) {
      setError(
        cause instanceof ApiError && cause.status === 404
          ? "Nicht mehr verfügbar."
          : "Nicht gespeichert. Versuch es noch einmal.",
      );
    } finally {
      setSaving(false);
    }
  };
  const level = own.level;

  return (
    <div className="notify-settings flex flex-col gap-5">
      <MarkRead target={target} onDone={onDone} />
      <section aria-labelledby="notify-mute" className="flex flex-col gap-2">
        <h3 id="notify-mute" className="text-sm font-semibold">
          Stummschalten
        </h3>
        {muted ? (
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-neutral-100 px-3 py-2 text-sm dark:bg-neutral-800">
            <span className="inline-flex items-center gap-2">
              <BellOffIcon size={16} />
              {own.muted_until
                ? `Stumm bis ${formatUntil(own.muted_until)}`
                : "Stumm, bis du es aufhebst"}
            </span>
            <button
              type="button"
              disabled={saving}
              className="notify-choice"
              onClick={() => void save({ level, muted: false })}
            >
              Aufheben
            </button>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2">
            {MUTES.map((mute) => (
              <button
                key={mute.label}
                type="button"
                disabled={saving}
                className="notify-choice"
                onClick={() =>
                  void save({
                    level,
                    muted: true,
                    ...(mute.minutes ? { mute_minutes: mute.minutes } : {}),
                  })
                }
              >
                {mute.label}
              </button>
            ))}
          </div>
        )}
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {target.dm
            ? "Stumm: kein Zähler, keine Benachrichtigung und kein Ton."
            : "Stumm: kein Punkt, keine Benachrichtigung und kein Ton. @Erwähnungen zählen weiter."}
          {serverMuted ? " Der ganze Server ist gerade stumm." : ""}
        </p>
      </section>
      {target.dm ? null : (
        <fieldset className="flex flex-col gap-1" disabled={saving}>
          <legend className="mb-1 text-sm font-semibold">
            Benachrichtigen bei
          </legend>
          {target.serverId ? (
            <LevelOption
              name={target.id}
              checked={level === null}
              label={`Wie der Server (${levelName(inherited)})`}
              onChange={() => void save({ level: null, muted: own.muted })}
            />
          ) : null}
          {LEVELS.map((row) => (
            <LevelOption
              key={row.level}
              name={target.id}
              checked={
                target.serverId
                  ? level === row.level
                  : (level ?? "all") === row.level
              }
              label={row.label}
              hint={row.hint}
              onChange={() =>
                void save({
                  // A server at "all" needs no row of its own.
                  level:
                    !target.serverId && row.level === "all" ? null : row.level,
                  muted: own.muted,
                  ...(own.muted && own.muted_until
                    ? { mute_minutes: minutesUntil(own.muted_until, now) }
                    : {}),
                })
              }
            />
          ))}
        </fieldset>
      )}
      {error ? (
        <p role="alert" className="text-sm text-red-700 dark:text-red-300">
          {error}
        </p>
      ) : null}
      <div className="flex justify-end">
        <GhostButton onClick={onDone} initialFocus>
          Fertig
        </GhostButton>
      </div>
    </div>
  );
}

/** "Als gelesen markieren" (#245): the conversation, the whole server or
 * every direct message, then the dialog closes. */
function MarkRead({
  target,
  onDone,
}: {
  target: NotificationTarget;
  onDone: () => void;
}) {
  const markRead = useMarkManyRead();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const label = target.allDms
    ? "Alle Direktnachrichten als gelesen markieren"
    : target.scope === "server"
      ? "Server als gelesen markieren"
      : "Als gelesen markieren";
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        disabled={busy}
        className="notify-choice self-start"
        onClick={() => {
          setBusy(true);
          setFailed(false);
          markRead(
            target.allDms
              ? { all: "dms" }
              : target.scope === "server"
                ? { server: target.id }
                : { channel: target.id },
          )
            .then(onDone)
            .catch(() => setFailed(true))
            .finally(() => setBusy(false));
        }}
      >
        {label}
      </button>
      {failed ? (
        <p role="alert" className="text-sm text-red-700 dark:text-red-300">
          Nicht gespeichert. Versuch es noch einmal.
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
    <label className="flex cursor-pointer items-start gap-3 rounded-lg px-2 py-1.5 hover:bg-neutral-100 dark:hover:bg-neutral-800">
      <input
        type="radio"
        name={`notify-level-${name}`}
        checked={checked}
        onChange={onChange}
        className="mt-1"
      />
      <span className="flex flex-col">
        <span className="text-sm font-medium">{label}</span>
        {hint ? (
          <span className="text-xs text-neutral-500 dark:text-neutral-400">
            {hint}
          </span>
        ) : null}
      </span>
    </label>
  );
}

function formatUntil(until: string): string {
  const date = new Date(until);
  const today = new Date();
  const time = date.toLocaleTimeString("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
  });
  return date.toDateString() === today.toDateString()
    ? `${time} Uhr`
    : `${date.toLocaleDateString("de-DE", { weekday: "short", day: "numeric", month: "short" })}, ${time} Uhr`;
}

/** The bell in a header: shows a mute, opens the dialog. */
export function NotificationBell({
  target,
  muted,
}: {
  target: NotificationTarget;
  muted: boolean;
}) {
  return (
    <button
      type="button"
      className="lr-header-icon notify-bell"
      aria-label={`Benachrichtigungen für ${target.name}${muted ? " (stumm)" : ""}`}
      title={muted ? "Stummgeschaltet" : "Benachrichtigungen"}
      onClick={() => openNotificationSettings(target)}
    >
      {muted ? <BellOffIcon size={16} /> : <BellIcon size={16} />}
    </button>
  );
}
