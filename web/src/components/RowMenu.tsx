// The menu of a channel, category, server or conversation in the
// navigation: opened by a right click, a held finger, Umschalt+F10 or the
// context-menu key on the entry, or by its "…" button. An entry can open a
// submenu beside the menu (inline in the phone's bottom sheet).
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";

import { pageZoom } from "../interface/zoom.ts";
import { notifyError } from "./toasts.ts";
import {
  BellIcon,
  BellOffIcon,
  CheckCheckIcon,
  CheckIcon,
  ChevronIcon,
  GearIcon,
  PencilIcon,
  PlusIcon,
  TrashIcon,
} from "./Icons.tsx";
import { useNotificationActions } from "./notificationActions.ts";
import type { NotificationTarget } from "./notificationTarget.ts";
import {
  notificationEntries,
  type LeafEntry,
  type MenuEntry,
  type MenuIcon,
} from "./rowMenuEntries.ts";
import { Popover } from "./ui/Popover.tsx";
import { POPOVER_SHEET_QUERY, submenuPosition } from "./ui/popoverPosition.ts";
import { useMediaQuery } from "./useMediaQuery.ts";
import "./shell.css";

const ICONS = {
  "check-check": CheckCheckIcon,
  bell: BellIcon,
  "bell-off": BellOffIcon,
  gear: GearIcon,
  pencil: PencilIcon,
  plus: PlusIcon,
  trash: TrashIcon,
} satisfies Record<MenuIcon, unknown>;

function EntryIcon({ icon }: { icon: MenuIcon | undefined }) {
  if (!icon) return null;
  const Icon = ICONS[icon];
  return <Icon size={18} />;
}

/** The submenu's padding and border: its first entry meets its opener. */
const SUBMENU_INSET = 9;

/** Marks a menu or submenu and its entries; the keys move within one. */
const LEVEL = "data-menu-level";
const ITEM = "data-menu-item";

function levelItems(level: Element): HTMLElement[] {
  return [...level.querySelectorAll<HTMLElement>(`[${ITEM}]`)].filter(
    (item) => item.closest(`[${LEVEL}]`) === level,
  );
}

export function RowMenu({
  open,
  onClose,
  anchor,
  label,
  entries,
}: {
  open: boolean;
  onClose: () => void;
  anchor: RefObject<HTMLElement | null>;
  /** "#allgemein", a server's or a person's name. */
  label: string;
  entries: MenuEntry[];
}) {
  const sheet = useMediaQuery(POPOVER_SHEET_QUERY);
  // The submenu open now, by its index in `entries`; `focus` when a key
  // opened it, so focus moves into it.
  const [shown, setShown] = useState<{ index: number; focus: boolean } | null>(
    null,
  );
  const sub = shown?.index ?? null;
  const triggers = useRef(new Map<number, HTMLButtonElement>());
  const closeSub = (focusTrigger: boolean) => {
    if (sub === null) return;
    if (focusTrigger) triggers.current.get(sub)?.focus();
    setShown(null);
  };
  const openSub = (index: number, focus: boolean) => setShown({ index, focus });
  const choose = (action: () => void) => () => {
    setShown(null);
    onClose();
    action();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target as HTMLElement;
    const level = target.closest(`[${LEVEL}]`);
    if (!level) return;
    const items = levelItems(level);
    const at = items.indexOf(target);
    const move = (index: number) => {
      event.preventDefault();
      items[(index + items.length) % items.length]?.focus();
    };
    switch (event.key) {
      case "ArrowDown":
        return move(at + 1);
      case "ArrowUp":
        return move(at < 0 ? -1 : at - 1);
      case "Home":
        return move(0);
      case "End":
        return move(-1);
      case "ArrowRight": {
        const index = Number(target.dataset.submenu);
        if (target.dataset.submenu === undefined || Number.isNaN(index)) return;
        event.preventDefault();
        openSub(index, true);
        return;
      }
      case "ArrowLeft":
        if (level.getAttribute(LEVEL) !== "sub") return;
        event.preventDefault();
        closeSub(true);
        return;
    }
  };

  return (
    <Popover
      open={open}
      onClose={() => {
        setShown(null);
        onClose();
      }}
      anchor={anchor}
      label={label}
      role="menu"
      className="shell-row-menu"
      onEscape={() => {
        if (sub === null) return false;
        closeSub(true);
        return true;
      }}
    >
      <div
        role="none"
        className="shell-row-menu-list"
        {...{ [LEVEL]: "" }}
        onKeyDown={onKeyDown}
      >
        {entries.map((entry, index) =>
          entry.kind === "submenu" ? (
            <div role="none" key={entry.label} className="shell-submenu-host">
              <button
                ref={(element) => {
                  if (element) triggers.current.set(index, element);
                  else triggers.current.delete(index);
                }}
                type="button"
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={sub === index}
                data-submenu={index}
                className="shell-menu-item"
                onPointerEnter={(event) => {
                  if (event.pointerType === "mouse" && !sheet)
                    openSub(index, false);
                }}
                onClick={(event) =>
                  sub === index && event.detail > 0
                    ? setShown(null)
                    : // A key (Enter, Space) opens it with focus inside.
                      openSub(index, event.detail === 0)
                }
                {...{ [ITEM]: "" }}
              >
                <EntryIcon icon={entry.icon} />
                <span className="shell-menu-label">{entry.label}</span>
                <ChevronIcon size={16} className="shell-menu-chevron" />
              </button>
              {sub === index ? (
                <Submenu
                  label={entry.label}
                  inline={sheet}
                  focusFirst={shown?.focus ?? false}
                >
                  {entry.entries.map((child, childIndex) => (
                    <Entry
                      key={childIndex}
                      entry={child}
                      choose={choose}
                      onPointerEnter={() => {}}
                    />
                  ))}
                </Submenu>
              ) : null}
            </div>
          ) : (
            <Entry
              key={
                entry.kind === "separator" ? `separator-${index}` : entry.label
              }
              entry={entry}
              choose={choose}
              onPointerEnter={() => {
                if (!sheet) setShown(null);
              }}
            />
          ),
        )}
      </div>
    </Popover>
  );
}

function Entry({
  entry,
  choose,
  onPointerEnter,
}: {
  entry: LeafEntry;
  choose: (action: () => void) => () => void;
  onPointerEnter: () => void;
}) {
  const hintId = useId();
  if (entry.kind === "separator")
    return <hr className="shell-menu-separator" />;
  if (entry.kind === "radio")
    return (
      <button
        type="button"
        role="menuitemradio"
        aria-checked={entry.checked}
        className="shell-menu-item"
        onClick={choose(entry.onSelect)}
        onPointerEnter={onPointerEnter}
        {...{ [ITEM]: "" }}
      >
        {entry.checked ? (
          <CheckIcon size={18} />
        ) : (
          <span className="shell-menu-icon-space" aria-hidden="true" />
        )}
        <span className="shell-menu-label">{entry.label}</span>
      </button>
    );
  // A disabled entry stays in the arrow keys' way, as menus do, but does
  // nothing. The hint is its description, not part of its name.
  return (
    <button
      type="button"
      role="menuitem"
      aria-describedby={entry.hint ? hintId : undefined}
      aria-disabled={entry.disabled || undefined}
      className={`shell-menu-item${entry.danger ? " is-danger" : ""}`}
      onClick={entry.disabled ? undefined : choose(entry.onSelect)}
      onPointerEnter={onPointerEnter}
      {...{ [ITEM]: "" }}
    >
      <EntryIcon icon={entry.icon} />
      <span className="shell-menu-label">
        {entry.label}
        {entry.hint ? (
          <small id={hintId} aria-hidden="true">
            {entry.hint}
          </small>
        ) : null}
      </span>
    </button>
  );
}

/** A submenu beside its menu, or inline under its entry in a sheet. */
function Submenu({
  label,
  inline,
  focusFirst,
  children,
}: {
  label: string;
  inline: boolean;
  /** Focus moves into it (a key opened it). */
  focusFirst: boolean;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = panel.current;
    // Right after the entry that opened it.
    const trigger = element?.previousElementSibling;
    if (!element || !trigger) return;
    if (focusFirst) levelItems(element)[0]?.focus();
    if (inline) return;
    // Beside the menu, again after the menu moves (Popover places it first).
    const place = () => {
      const menu = trigger.closest(".ui-popover") ?? trigger;
      const position = submenuPosition(
        menu.getBoundingClientRect(),
        trigger.getBoundingClientRect(),
        { width: element.offsetWidth, height: element.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
        SUBMENU_INSET,
        pageZoom(element),
      );
      element.style.top = `${position.top}px`;
      element.style.left = `${position.left}px`;
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [inline, focusFirst]);
  return (
    <div
      ref={panel}
      role="menu"
      aria-label={label}
      className={`shell-submenu${inline ? " is-inline" : ""}`}
      {...{ [LEVEL]: "sub" }}
    >
      {children}
    </div>
  );
}

/**
 * The notification entries of a server, channel or conversation: mark it
 * read, mute it, its level, and the dialog with all of it. `extra` follows
 * after a separator (a channel's edit and delete).
 */
export function NotificationRowMenu({
  target,
  anchor,
  onClose,
  extra = [],
  onSettings,
}: {
  target: NotificationTarget;
  anchor: RefObject<HTMLElement | null>;
  onClose: () => void;
  extra?: MenuEntry[];
  /** Opens the dialog another way, e.g. after the phone drawer closed. */
  onSettings?: () => void;
}) {
  const actions = useNotificationActions(target);
  const entries = notificationEntries(
    target,
    onSettings ? { ...actions, openSettings: onSettings } : actions,
    (what) => (error) => notifyError(error, what),
  );
  return (
    <RowMenu
      open
      onClose={onClose}
      anchor={anchor}
      label={target.name}
      entries={
        extra.length > 0
          ? [...entries, { kind: "separator" }, ...extra]
          : entries
      }
    />
  );
}
