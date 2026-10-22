// The desktop app's own system notifications (desktop/app/src/notify.rs):
// Windows toasts, and on Linux the desktop's notification service (mako,
// KDE, GNOME, dunst). They replace the webview's web notifications, which
// carry the webview's name and do not lead back to the window reliably. What
// is announced and how it reads stays the page's (pwa/notifications.ts). An
// older app without these commands refuses them, and the page keeps the web
// notifications there.
import {
  invokeNative,
  isDesktopApp,
  nativeBridge,
} from "../voice/native/bridge.ts";

type Info = { available?: unknown };
type NotifyEvent = { type?: unknown };

let checked: Promise<boolean> | undefined;

/** Whether this page's notifications go through the app; once per page. */
export function desktopNotifications(): Promise<boolean> {
  if (!isDesktopApp()) return Promise.resolve(false);
  checked ??= invokeNative<Info>("notify_info").then(
    (info) => info?.available === true,
    () => false,
  );
  return checked;
}

/**
 * Show one, or replace the one with the same tag. `onClick` runs after the
 * app has brought its window to the front. False when the app refused it.
 */
export async function showDesktopNotification(
  notice: { tag: string; title: string; body: string },
  onClick: () => void,
): Promise<boolean> {
  try {
    const events = await nativeBridge().channel<NotifyEvent>((event) => {
      if (event?.type === "click") onClick();
    });
    await invokeNative("notify_show", { ...notice, events });
    return true;
  } catch {
    return false;
  }
}

/** Take down the one with `tag`, or with `prefix` all whose tag starts so. */
export async function closeDesktopNotifications(
  tag: string,
  prefix: boolean,
): Promise<void> {
  if (!(await desktopNotifications())) return;
  await invokeNative("notify_close", { tag, prefix }).catch(() => {});
}

/** Tests: ask the app again. */
export function forgetDesktopNotificationsForTests(): void {
  checked = undefined;
}
