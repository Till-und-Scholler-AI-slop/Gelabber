// The technical details that go along with an entry: app, system, version,
// language and page. The dialog shows exactly these; nothing else is sent.

import type { DeviceOs } from "../auth/devices.ts";
import type { Locale } from "../i18n/locale.ts";
import { APP_VERSION } from "../version.ts";
import type { FeedbackContext } from "./types.ts";

/** The longest `page` the API takes. */
export const PAGE_MAX = 300;

/**
 * The system from a User-Agent, by the rules and in the order of the API's
 * `auth/device.rs`: Android before Linux, iPhone before Mac.
 */
export function osFromUserAgent(agent: string): DeviceOs {
  const any = (needles: string[]) =>
    needles.some((needle) => agent.includes(needle));
  if (any(["Android"])) return "android";
  if (any(["iPhone", "iPad", "iPod"])) return "ios";
  if (any(["Windows"])) return "windows";
  if (any(["Mac OS X", "Macintosh"])) return "macos";
  if (any(["Linux", "X11"])) return "linux";
  return "other";
}

/** A path without query and fragment, cut to what the API takes. */
export function feedbackPage(path: string): string {
  const bare = path.replace(/[?#].*$/s, "");
  const page = bare.startsWith("/") && !bare.startsWith("//") ? bare : "/";
  return Array.from(page).slice(0, PAGE_MAX).join("");
}

/** What the web app sends along from `pathname`. */
export function feedbackContext(
  pathname: string,
  locale: Locale,
  agent: string = typeof navigator === "undefined" ? "" : navigator.userAgent,
): FeedbackContext {
  return {
    client: "web",
    os: osFromUserAgent(agent),
    app_version: APP_VERSION,
    locale,
    page: feedbackPage(pathname),
  };
}

/** The page the dialog was opened on, as the address bar has it. */
export function currentPath(): string {
  return typeof window === "undefined" ? "/" : window.location.pathname;
}
