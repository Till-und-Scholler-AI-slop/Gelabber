// What this client can capture and where it can play. One rule for every
// control: the browser answers through the APIs it has. Phone browsers have
// no getDisplayMedia, an insecure origin has no mediaDevices at all.

function browserCaptures(method: "getDisplayMedia" | "getUserMedia"): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof navigator.mediaDevices?.[method] === "function"
  );
}

/** Live: a display capture. */
export function canShareScreen(): boolean {
  return browserCaptures("getDisplayMedia");
}

/** A Live can carry sound: the browser offers it in its own picker, or
 * answers with video only. */
export function canShareAppAudio(): boolean {
  return canShareScreen();
}

export function canUseCamera(): boolean {
  return browserCaptures("getUserMedia");
}

/** The speaker can be chosen only where audio elements have setSinkId. */
export function canChooseSpeaker(): boolean {
  return (
    typeof HTMLMediaElement !== "undefined" &&
    typeof HTMLMediaElement.prototype.setSinkId === "function"
  );
}

export type Capabilities = {
  screen: boolean;
  appAudio: boolean;
  camera: boolean;
};

/** The same rule for components. */
export function useCapabilities(): Capabilities {
  return {
    screen: canShareScreen(),
    appAudio: canShareAppAudio(),
    camera: canUseCamera(),
  };
}
