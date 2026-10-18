// Unread state outside the page: the tab title, the favicon, the installed
// app's badge and the desktop app's taskbar and tray (issues #239, #240,
// #246). One summary drives all of them.
import { useEffect } from "react";

import { reportDesktopUnread, type UnreadMark } from "./desktopShell.ts";
import {
  type ReadState,
  useReadState,
  useReadingChannel,
} from "./readState.ts";

/** What is unread, without the conversation being read right now. */
export function unreadMark(
  rows: readonly Pick<ReadState, "channel_id" | "unread_count">[],
  reading: string | null,
): UnreadMark {
  const count = rows.reduce(
    (total, row) =>
      total + (row.channel_id === reading ? 0 : Math.max(0, row.unread_count)),
    0,
  );
  return { count, any: count > 0 };
}

// What this module puts in front of a title: "(3) " or "• ".
const TITLE_MARK = /^(?:\(\d+\+?\)|•) /;

/** `(3) Gelabber`, `(99+) Gelabber` from 100 on, `• Gelabber` for unread
 * messages without a number, the plain title otherwise. A title some page
 * set in between keeps its text. */
export function titleWith(title: string, mark: UnreadMark): string {
  const base = title.replace(TITLE_MARK, "");
  if (mark.count > 0)
    return `(${mark.count > 99 ? "99+" : mark.count}) ${base}`;
  return mark.any ? `• ${base}` : base;
}

type BadgeNavigator = {
  setAppBadge?: (count?: number) => Promise<void>;
  clearAppBadge?: () => Promise<void>;
};

/** The installed app's icon (PWA). Browsers without it, or a tab that is not
 * installed, ignore it. */
export function setAppBadge(
  count: number,
  nav: BadgeNavigator | undefined = typeof navigator === "undefined"
    ? undefined
    : (navigator as BadgeNavigator),
): void {
  try {
    const done = count > 0 ? nav?.setAppBadge?.(count) : nav?.clearAppBadge?.();
    done?.catch(() => {});
  } catch {
    // Not allowed here (an insecure origin, a sandboxed frame).
  }
}

let originalIcon: string | null = null;
let dottedIcon: Promise<string | null> | null = null;
let iconWanted = false;

function iconLink(): HTMLLinkElement | null {
  return document.querySelector<HTMLLinkElement>('link[rel="icon"]');
}

/** The favicon with a red dot in its corner, drawn once. */
function drawDotted(source: string): Promise<string | null> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => {
      try {
        const size = 64;
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = size;
        const context = canvas.getContext("2d");
        if (!context) return resolve(null);
        context.drawImage(image, 0, 0, size, size);
        const radius = size * 0.24;
        const centre = size - radius - 2;
        // A clear ring sets the dot off from the icon.
        context.globalCompositeOperation = "destination-out";
        context.beginPath();
        context.arc(centre, centre, radius + 3, 0, Math.PI * 2);
        context.fill();
        context.globalCompositeOperation = "source-over";
        context.fillStyle = "#e5484d";
        context.beginPath();
        context.arc(centre, centre, radius, 0, Math.PI * 2);
        context.fill();
        resolve(canvas.toDataURL("image/png"));
      } catch {
        resolve(null);
      }
    };
    image.onerror = () => resolve(null);
    image.src = source;
  });
}

function showIconDot(on: boolean): void {
  const link = iconLink();
  if (!link) return;
  iconWanted = on;
  originalIcon ??= link.getAttribute("href");
  if (!on) {
    if (originalIcon) link.setAttribute("href", originalIcon);
    return;
  }
  if (!originalIcon) return;
  dottedIcon ??= drawDotted(originalIcon);
  void dottedIcon.then((dotted) => {
    // Read in the meantime: the plain icon stays.
    if (dotted && iconWanted) iconLink()?.setAttribute("href", dotted);
  });
}

function apply(mark: UnreadMark): void {
  document.title = titleWith(document.title, mark);
  showIconDot(mark.any);
  setAppBadge(mark.count);
  reportDesktopUnread(mark);
}

/** Mounted once per signed-in account; signing out clears everything. */
export function useUnreadIndicators(): void {
  const reading = useReadingChannel();
  const { data } = useReadState();
  const { count, any } = unreadMark(data ?? [], reading);
  useEffect(() => {
    apply({ count, any });
  }, [count, any]);
  useEffect(() => () => apply({ count: 0, any: false }), []);
}
