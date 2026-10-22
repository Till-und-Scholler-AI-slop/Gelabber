// Strg/⌘+K: jump to any server, channel or direct message by typing part
// of its name. With nothing typed it lists what is unread first.

import { useNavigate } from "@tanstack/react-router";
import { useMemo, useState, type CSSProperties } from "react";

import { useUserId } from "../auth/scope.ts";
import { useDms } from "../dms/queries.ts";
import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadges } from "../messages/useBadge.ts";
import {
  lastChannelsFor,
  railChannel,
  useLastChannel,
} from "../servers/lastChannel.ts";
import { useServerDetails, useServers } from "../servers/queries.ts";
import { useVoiceRoster } from "../voice/roster.ts";
import { Avatar } from "./Avatar.tsx";
import { HashIcon, SpeakerIcon } from "./Icons.tsx";
import { Modal } from "./Modal.tsx";
import { Picker, type PickerOption } from "./Picker.tsx";
import {
  rankEntries,
  switchEntries,
  type SwitchEntry,
  type SwitchTarget,
} from "./quickSwitch.ts";
import { isMac, shortcutList } from "./shellShortcuts.ts";
import { tileInitials, tileTint } from "./initials.ts";
import "./shell.css";

export function QuickSwitcher({
  open,
  onClose,
  current,
}: {
  open: boolean;
  onClose: () => void;
  /** The conversation open now, left out of the list. */
  current: string | undefined;
}) {
  return (
    <Modal open={open} onClose={onClose} title="Wechseln zu">
      {open ? <SwitcherBody onClose={onClose} current={current} /> : null}
    </Modal>
  );
}

function SwitcherBody({
  onClose,
  current,
}: {
  onClose: () => void;
  current: string | undefined;
}) {
  const navigate = useNavigate();
  const userId = useUserId();
  const [query, setQuery] = useState("");
  const { data: servers } = useServers();
  const details = useServerDetails((servers ?? []).map((server) => server.id));
  const { data: dms } = useDms();
  const badges = useBadges();
  const byUser = useLastChannel((s) => s.byUser);
  const entries = useMemo(
    () =>
      switchEntries({
        servers: servers ?? [],
        details,
        dms: dms ?? [],
        badges,
        current,
      }),
    [servers, details, dms, badges, current],
  );
  const options = rankEntries(entries, query).map(
    (entry): PickerOption<SwitchTarget> => ({
      id: entry.id,
      value: entry.target,
      label: entry.kind === "text" ? `#${entry.label}` : entry.label,
      detail: entry.detail,
      icon: <EntryIcon entry={entry} />,
      trailing: <EntryBadge entry={entry} />,
    }),
  );

  const go = (target: SwitchTarget) => {
    onClose();
    if (target.kind === "dm") {
      void navigate({
        to: "/d/$channelId",
        params: { channelId: target.channelId },
      });
      return;
    }
    const channelId =
      target.kind === "channel"
        ? target.channelId
        : railChannel(
            lastChannelsFor(byUser, userId)[target.serverId],
            Object.keys(
              useVoiceRoster.getState().byServer[target.serverId] ?? {},
            ).length > 0,
          );
    void (channelId
      ? navigate({
          to: "/s/$serverId/c/$channelId",
          params: { serverId: target.serverId, channelId },
        })
      : navigate({
          to: "/s/$serverId",
          params: { serverId: target.serverId },
        }));
  };

  return (
    <>
      <Picker
        label="Server, Kanal oder Unterhaltung"
        placeholder="Wohin möchtest du?"
        query={query}
        onQuery={setQuery}
        options={options}
        onPick={(option) => go(option.value)}
        empty={`Nichts gefunden für „${query.trim()}“.`}
      />
      <p className="shell-picker-hint" aria-hidden="true">
        ↑ ↓ auswählen · Enter öffnen · Esc schließen
      </p>
    </>
  );
}

function EntryIcon({ entry }: { entry: SwitchEntry }) {
  if (entry.kind === "dm")
    return <Avatar name={entry.label} url={entry.avatar ?? null} decorative />;
  if (entry.kind === "server" && entry.target.kind === "server")
    return (
      <span
        className="shell-picker-tile"
        style={
          { "--server-tile": tileTint(entry.target.serverId) } as CSSProperties
        }
      >
        {tileInitials(entry.label)}
      </span>
    );
  return entry.kind === "voice" ? (
    <SpeakerIcon size={18} />
  ) : (
    <HashIcon size={18} />
  );
}

function EntryBadge({ entry }: { entry: SwitchEntry }) {
  const target = entry.target;
  if (target.kind === "server")
    return (
      <UnreadBadge serverId={target.serverId} className="shell-picker-badge" />
    );
  if (entry.kind === "voice") return null;
  return (
    <UnreadBadge channelId={target.channelId} className="shell-picker-badge" />
  );
}

/** Strg/⌘+/: every shortcut of the workspace. */
export function ShortcutSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Modal open={open} onClose={onClose} title="Tastenkürzel">
      <dl className="shortcut-list">
        {shortcutList(isMac()).map((shortcut) => (
          <div key={shortcut.label}>
            <dt>
              {shortcut.keys.map((key) => (
                <kbd key={key}>{key}</kbd>
              ))}
            </dt>
            <dd>{shortcut.label}</dd>
          </div>
        ))}
      </dl>
    </Modal>
  );
}
