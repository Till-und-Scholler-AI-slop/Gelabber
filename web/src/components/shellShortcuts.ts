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

export type Shortcut = { keys: string[]; label: string };
export type ShortcutGroup = { title: string; shortcuts: Shortcut[] };

/** What the shortcut sheet shows, in its order: the workspace's keys, then
 * the call's (voice/callShortcuts.ts, kinoKey, VoiceTile). `pushToTalk` is
 * the key push-to-talk holds, as the settings name it, when it is on. */
export function shortcutGroups(
  mac: boolean,
  pushToTalk: string | null = null,
): ShortcutGroup[] {
  const { mod, alt, shift } = keyNames(mac);
  return [
    {
      title: "Allgemein",
      shortcuts: [
        { keys: [mod, "K"], label: "Server, Kanal oder Unterhaltung öffnen" },
        { keys: [mod, "F"], label: "In der Unterhaltung suchen" },
        { keys: [alt, "↑ / ↓"], label: "Vorheriger oder nächster Kanal" },
        {
          keys: [alt, shift, "↑ / ↓"],
          label: "Vorheriger oder nächster ungelesener Kanal",
        },
        { keys: [mod, alt, "↑ / ↓"], label: "Vorheriger oder nächster Server" },
        // Whatever is open takes Esc first (MessagePane).
        {
          keys: ["Esc"],
          label: "Schließen, sonst Unterhaltung als gelesen markieren",
        },
        { keys: [shift, "Esc"], label: "Ganzen Server als gelesen markieren" },
        { keys: [mod, "/"], label: "Diese Übersicht" },
      ],
    },
    {
      title: "Im Sprachkanal",
      shortcuts: [
        { keys: [mod, shift, "M"], label: "Stummschaltung ein oder aus" },
        { keys: [mod, shift, "D"], label: "Taubschaltung ein oder aus" },
        ...(pushToTalk
          ? [{ keys: [pushToTalk], label: "Zum Sprechen gedrückt halten" }]
          : []),
        { keys: ["K"], label: "Kino ein oder aus" },
        {
          keys: ["M / Leertaste"],
          label: "Im Vollbild: Stummschaltung ein oder aus",
        },
      ],
    },
  ];
}
