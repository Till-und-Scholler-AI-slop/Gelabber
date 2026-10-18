// The desktop app's window, taskbar and tray (desktop/app/src/shell.rs). In a
// browser all of this does nothing; an older app without these commands
// refuses them, which is no error for the page either.
import { invokeNative, isDesktopApp } from "../voice/native/bridge.ts";

export type DesktopShellInfo = { tray: boolean; close_to_tray: boolean };

/** What the tray can do here; null outside the app or in an older one. */
export async function desktopShellInfo(): Promise<DesktopShellInfo | null> {
  if (!isDesktopApp()) return null;
  try {
    return await invokeNative<DesktopShellInfo>("shell_info");
  } catch {
    return null;
  }
}

export async function setCloseToTray(enabled: boolean): Promise<boolean> {
  try {
    await invokeNative("shell_close_to_tray", { enabled });
    return true;
  } catch {
    return false;
  }
}

/** The app window in front, after a click on a notification. */
export function focusDesktopWindow(): void {
  if (!isDesktopApp()) return;
  invokeNative("shell_focus").catch(() => {});
}

export type UnreadMark = {
  /** The number for badges; 0 shows none. */
  count: number;
  /** Something is unread at all, counted or not. */
  any: boolean;
};

let reported: UnreadMark = { count: 0, any: false };

/** Taskbar, launcher and tray follow the page's unread state. */
export function reportDesktopUnread(mark: UnreadMark): void {
  reported = mark;
  if (!isDesktopApp()) return;
  invokeNative("shell_unread", { ...mark, attention: false }).catch(() => {});
}

/** Once per call: a direct message arrived while the window is not in front.
 * The app flashes the taskbar button (Windows) or marks the window urgent
 * (Linux) until it gets the focus, and leaves a focused window alone. */
export function requestDesktopAttention(): void {
  if (!isDesktopApp()) return;
  invokeNative("shell_unread", { ...reported, attention: true }).catch(
    () => {},
  );
}
