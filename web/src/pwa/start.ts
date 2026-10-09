import { followNotificationTaps } from "./notifications.ts";
import { registerServiceWorker } from "./register.ts";

/**
 * What only the production build in a browser starts: the service worker
 * (the offline page, and on phones the notifications, which go through its
 * registration) and the change of route a tap on such a notification leads
 * to. Vite's dev server and the desktop app get neither. Answers whether it
 * started them.
 */
export function startPwa(app: {
  desktop: boolean;
  user: () => string | undefined;
  navigate: (to: { href: string }) => unknown;
}): boolean {
  if (!import.meta.env.PROD || app.desktop) return false;
  registerServiceWorker();
  followNotificationTaps({
    user: app.user,
    open: (path) => void app.navigate({ href: path }),
  });
  return true;
}
