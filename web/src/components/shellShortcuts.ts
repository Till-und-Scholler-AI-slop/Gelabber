// The workspace's keyboard shortcuts: which key does what, and where each
// one moves to. Apart from the DOM so it can be tested in node.

export type ShellShortcut =
  | { kind: "switcher" }
  | { kind: "help" }
  | { kind: "channel" | "unread" | "server"; step: 1 | -1 };

type Keys = {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
};

/**
 * Strg/⌘+K: quick switcher. Strg/⌘+/: this list. Alt+↑/↓: previous or
 * next channel, with Umschalt the previous or next unread one, with
 * Strg/⌘ the previous or next server.
 */
export function shellShortcut(event: Keys): ShellShortcut | null {
  const mod = event.ctrlKey || event.metaKey;
  if (
    mod &&
    !event.altKey &&
    !event.shiftKey &&
    event.key.toLowerCase() === "k"
  )
    return { kind: "switcher" };
  // "/" needs Umschalt on German keyboards; it is the same key either way.
  if (mod && !event.altKey && event.key === "/") return { kind: "help" };
  if (!event.altKey) return null;
  const step =
    event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : null;
  if (step === null) return null;
  if (mod) return event.shiftKey ? null : { kind: "server", step };
  return { kind: event.shiftKey ? "unread" : "channel", step };
}

/** The neighbour of `index` in a list of `count`, or `null` at its end.
 * From outside the list (-1) the first step goes to either end. */
export function stepIndex(
  count: number,
  index: number,
  step: 1 | -1,
): number | null {
  if (count === 0) return null;
  if (index < 0) return step === 1 ? 0 : count - 1;
  const next = index + step;
  return next < 0 || next >= count ? null : next;
}

/** The next unread entry after `index` in direction `step`, round the
 * end of the list; `null` when nothing else is unread. */
export function nextUnread(
  unread: readonly boolean[],
  index: number,
  step: 1 | -1,
): number | null {
  const count = unread.length;
  for (let offset = 1; offset <= count; offset += 1) {
    const at = (((index + step * offset) % count) + count) % count;
    if (at !== index && unread[at]) return at;
  }
  return null;
}

export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const platform =
    (navigator as { userAgentData?: { platform?: string } }).userAgentData
      ?.platform ?? navigator.platform;
  return /mac|iphone|ipad/i.test(platform);
}

/** Key names as the keyboard prints them. */
export function keyNames(mac: boolean) {
  return mac
    ? { mod: "⌘", alt: "⌥", shift: "⇧" }
    : { mod: "Strg", alt: "Alt", shift: "Umschalt" };
}

/** The list the shortcut sheet shows, in its order. */
export function shortcutList(
  mac: boolean,
): { keys: string[]; label: string }[] {
  const { mod, alt, shift } = keyNames(mac);
  return [
    { keys: [mod, "K"], label: "Server, Kanal oder Unterhaltung öffnen" },
    { keys: [mod, "F"], label: "In der Unterhaltung suchen" },
    { keys: [alt, "↑ / ↓"], label: "Vorheriger oder nächster Kanal" },
    {
      keys: [alt, shift, "↑ / ↓"],
      label: "Vorheriger oder nächster ungelesener Kanal",
    },
    { keys: [mod, alt, "↑ / ↓"], label: "Vorheriger oder nächster Server" },
    { keys: ["Esc"], label: "Unterhaltung als gelesen markieren" },
    { keys: [shift, "Esc"], label: "Ganzen Server als gelesen markieren" },
    { keys: ["K"], label: "Kino im Sprachkanal ein oder aus" },
    { keys: [mod, "/"], label: "Diese Übersicht" },
  ];
}
