// Who watches a Live (#521): the count on its caption and on the own Live's
// call bar button, and the list both open. Everyone else in the channel is
// listed apart; the own Live's list ends with the way to stop it.

import { useRef, useState, type ReactNode, type RefObject } from "react";

import { useT } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import { formatNumber } from "../i18n/format.ts";
import type { Member } from "../servers/types.ts";
import { Avatar } from "../components/Avatar.tsx";
import { EyeIcon, StopIcon } from "../components/Icons.tsx";
import { Popover, type PopoverPlacement } from "../components/ui/Popover.tsx";
import { toggleShare } from "./session.ts";
import {
  useLiveWatchers,
  watchersTitle,
  type LiveWatcherList,
  type WatchPerson,
} from "./useLiveWatchers.ts";

/** The list: who watches, who in the channel does not, and on the own Live
 * "Live beenden" and whatever `extra` the caller adds under it. */
export function LiveWatchersPopover({
  open,
  onClose,
  anchor,
  placement,
  own,
  ownerName,
  channelName,
  list,
  extra,
}: {
  open: boolean;
  onClose: () => void;
  anchor: RefObject<HTMLElement | null>;
  placement: PopoverPlacement;
  own: boolean;
  /** The owner's name, without "(du)". */
  ownerName: string;
  channelName: string;
  list: LiveWatcherList;
  extra?: ReactNode;
}) {
  const t = useT(voice);
  const title = watchersTitle(t, list.watching.length);
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchor={anchor}
      label={title}
      placement={placement}
      className="voice-watchers"
    >
      <div className="voice-watchers-head">
        <strong>{title}</strong>
        <small>
          {own
            ? t("watchers.ownSubtitle", { channel: channelName })
            : t("watchers.subtitle", { name: ownerName, channel: channelName })}
        </small>
      </div>
      {list.watching.length > 0 ? <People people={list.watching} /> : null}
      {list.notWatching.length > 0 ? (
        <section className="voice-watchers-others">
          <h3>{t("watchers.notWatching")}</h3>
          <People people={list.notWatching} />
        </section>
      ) : null}
      {own ? (
        <>
          <div role="separator" className="voice-menu-separator" />
          <button
            type="button"
            className="voice-menu-item is-danger voice-watchers-end"
            onClick={() => {
              onClose();
              toggleShare();
            }}
          >
            <StopIcon size={16} />
            <span>
              <strong>{t("live.end")}</strong>
            </span>
          </button>
          {extra}
        </>
      ) : null}
    </Popover>
  );
}

function People({ people }: { people: WatchPerson[] }) {
  return (
    <ul className="voice-watchers-people">
      {people.map((person) => (
        <li key={person.id}>
          <Avatar name={person.name} url={person.url} decorative />
          <span>{person.name}</span>
        </li>
      ))}
    </ul>
  );
}

/** The eye and the count on a Live's caption; a press lists the watchers. */
export function LiveWatchCount({
  serverId,
  channelId,
  channelName,
  owner,
  ownerName,
  own,
  members,
}: {
  serverId: string;
  channelId: string;
  channelName: string;
  owner: string;
  ownerName: string;
  own: boolean;
  members: readonly Member[] | undefined;
}) {
  const t = useT(voice);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const list = useLiveWatchers({ serverId, channelId, owner, members });
  const count = list.watching.length;
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="voice-live-count"
        aria-label={
          count === 0
            ? t("watchers.countLabelNone")
            : t("watchers.countLabel", { count })
        }
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <EyeIcon size={14} />
        <span>{formatNumber(count)}</span>
      </button>
      <LiveWatchersPopover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        placement="top-start"
        own={own}
        ownerName={ownerName}
        channelName={channelName}
        list={list}
      />
    </>
  );
}
