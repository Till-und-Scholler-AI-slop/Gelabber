// What this client can capture and where it can play. One rule for every
// control: the desktop app answers through its feature list, a browser
// through the APIs it has. Phone browsers have no getDisplayMedia, an
// insecure origin has no mediaDevices at all.

import { useSyncExternalStore } from "react";

import { isDesktopApp } from "./native/bridge.ts";
import {
  hasNativeFeature,
  nativeFeatures,
  subscribeNativeFeatures,
} from "./native/features.ts";

function browserCaptures(method: "getDisplayMedia" | "getUserMedia"): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof navigator.mediaDevices?.[method] === "function"
  );
}

/** Screen share and Go Live; both are display captures. */
export function canShareScreen(): boolean {
  return isDesktopApp()
    ? hasNativeFeature("screen")
    : browserCaptures("getDisplayMedia");
}

/** A share can carry sound. The desktop app captures an application's; a
 * browser offers it in its own picker, or answers with video only. */
export function canShareAppAudio(): boolean {
  if (!canShareScreen()) return false;
  return !isDesktopApp() || hasNativeFeature("app-audio");
}

export function canUseCamera(): boolean {
  return isDesktopApp()
    ? hasNativeFeature("camera")
    : browserCaptures("getUserMedia");
}

/** The speaker can be chosen: in the desktop app from the native core's own
 * list, in a browser only where audio elements have setSinkId. */
export function canChooseSpeaker(): boolean {
  if (isDesktopApp()) return true;
  return (
    typeof HTMLMediaElement !== "undefined" &&
    typeof HTMLMediaElement.prototype.setSinkId === "function"
  );
}

export type Capabilities = {
  screen: boolean;
  appAudio: boolean;
  camera: boolean;
  /** These are the desktop app's own answers. People look for a missing
   * capture there, so its control says so; a browser that cannot capture
   * simply has no such control. */
  desktop: boolean;
};

/** The same rule for components. In the desktop app everything is off until
 * the app has answered; the component then renders again. */
export function useCapabilities(): Capabilities {
  const features = useSyncExternalStore(
    subscribeNativeFeatures,
    nativeFeatures,
    nativeFeatures,
  );
  return {
    screen: canShareScreen(),
    appAudio: canShareAppAudio(),
    camera: canUseCamera(),
    desktop: isDesktopApp() && features !== null,
  };
}
