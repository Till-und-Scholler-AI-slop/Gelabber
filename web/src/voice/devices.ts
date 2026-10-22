// The microphones, speakers and cameras to choose from, listed as soon as a
// picker shows and again whenever one is plugged in or out. A browser names
// them only after a permission: the microphone's is asked for on request,
// the camera's only from the camera field.

import { useCallback, useEffect, useState } from "react";

import { isDesktopApp } from "./native/bridge.ts";
import {
  listMediaDevices,
  type DeviceList,
  type DeviceOption,
} from "./settings.ts";

const EMPTY: DeviceList = { audioinput: [], audiooutput: [], videoinput: [] };

/** Chrome's stand-ins for the system's choice; "" means that already. */
const DEFAULT_ID = "default";

/** Whether the browser keeps this kind's names back until a permission:
 * then it lists no device, or devices without a label. */
export function needsDeviceLabels(options: readonly DeviceOption[]): boolean {
  return (
    options.length === 0 ||
    options.every((option) => option.label === option.id)
  );
}

/** The device the system picks, by name, where the browser says which. */
export function defaultDeviceName(
  options: readonly DeviceOption[],
): string | null {
  const stand = options.find((option) => option.id === DEFAULT_ID);
  if (stand && stand.label !== stand.id)
    return stand.label.replace(/^(default|standard)\s*[-–:]\s*/i, "");
  return null;
}

/** What a device field offers: the system's choice first ("Standard –
 * Headset"), then each device once, and a saved one that is not listed
 * under its own entry. */
export function deviceChoices(
  options: readonly DeviceOption[],
  value: string,
): { value: string; options: { id: string; label: string }[] } {
  const desktop = isDesktopApp();
  const named = defaultDeviceName(options);
  const standard = desktop
    ? "Systemstandard"
    : named
      ? `Standard – ${named}`
      : "Standard";
  const devices = options.filter((option) => option.id !== DEFAULT_ID);
  const current = value === DEFAULT_ID ? "" : value;
  return {
    value: current,
    options: [
      { id: "", label: standard },
      ...(current && !devices.some((option) => option.id === current)
        ? [{ id: current, label: "Gewähltes Gerät" }]
        : []),
      ...devices,
    ],
  };
}

type Devices = Pick<
  MediaDevices,
  "getUserMedia" | "addEventListener" | "removeEventListener"
>;

/** Lists now and again on every plug or unplug, until the returned stop. */
export function watchMediaDevices(
  media: Devices | undefined,
  list: () => void,
): () => void {
  list();
  media?.addEventListener?.("devicechange", list);
  return () => media?.removeEventListener?.("devicechange", list);
}

/** The one permission that makes a browser name a kind: the microphone's
 * for audio, the camera's only when asked for cameras. Nothing is kept. */
export async function revealDeviceNames(
  media: Devices | undefined,
  kind: "audio" | "video",
): Promise<boolean> {
  if (!media?.getUserMedia) return false;
  try {
    const stream = await media.getUserMedia(
      kind === "audio" ? { audio: true } : { video: true },
    );
    stream.getTracks().forEach((track) => track.stop());
    return true;
  } catch {
    return false;
  }
}

const mediaDevices = (): Devices | undefined =>
  typeof navigator === "undefined" ? undefined : navigator.mediaDevices;

/** The device lists, kept current while the caller is shown; `loaded` once
 * the first listing is in. `reveal` asks for the permission that makes a
 * browser name a kind. */
export function useMediaDevices(): {
  devices: DeviceList;
  loaded: boolean;
  reveal: (kind: "audio" | "video") => Promise<boolean>;
} {
  const [devices, setDevices] = useState<DeviceList | null>(null);
  const refresh = useCallback(() => {
    void listMediaDevices()
      .then(setDevices)
      .catch(() => setDevices(EMPTY));
  }, []);
  useEffect(() => watchMediaDevices(mediaDevices(), refresh), [refresh]);
  const reveal = useCallback(
    async (kind: "audio" | "video") => {
      if (isDesktopApp()) return false;
      const ok = await revealDeviceNames(mediaDevices(), kind);
      if (ok) refresh();
      return ok;
    },
    [refresh],
  );
  return { devices: devices ?? EMPTY, loaded: devices !== null, reveal };
}
