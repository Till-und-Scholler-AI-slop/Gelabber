import { afterEach, describe, expect, it, vi } from "vitest";

import {
  defaultDeviceName,
  deviceChoices,
  needsDeviceLabels,
  revealDeviceNames,
  watchMediaDevices,
} from "./devices.ts";

const headset = { id: "h1", label: "Headset" };
const laptop = { id: "l1", label: "Eingebautes Mikrofon" };
const chromeDefault = { id: "default", label: "Default - Headset" };

afterEach(() => vi.unstubAllGlobals());

describe("device names", () => {
  it("knows when a browser keeps them back", () => {
    expect(needsDeviceLabels([])).toBe(true);
    expect(needsDeviceLabels([{ id: "x1", label: "x1" }])).toBe(true);
    expect(needsDeviceLabels([headset])).toBe(false);
  });

  it("names the system's choice where the browser says which it is", () => {
    expect(defaultDeviceName([chromeDefault, headset])).toBe("Headset");
    expect(defaultDeviceName([headset, laptop])).toBeNull();
  });

  it("offers the system's choice once, by name, then the devices", () => {
    const choices = deviceChoices([chromeDefault, headset, laptop], "");
    expect(choices.options).toEqual([
      { id: "", label: "Standard – Headset" },
      headset,
      laptop,
    ]);
    // Chrome's own stand-in is the system's choice already.
    expect(deviceChoices([chromeDefault, headset], "default").value).toBe("");
  });

  it("keeps a chosen device that is not listed", () => {
    expect(deviceChoices([headset], "gone").options).toEqual([
      { id: "", label: "Standard" },
      { id: "gone", label: "Gewähltes Gerät" },
      headset,
    ]);
  });
});

function fakeDevices() {
  const listeners = new Set<() => void>();
  return {
    listeners,
    getUserMedia: vi.fn(async () => ({
      getTracks: () => [{ stop: vi.fn() }],
    })),
    addEventListener: vi.fn((_: string, listener: () => void) =>
      listeners.add(listener),
    ),
    removeEventListener: vi.fn((_: string, listener: () => void) =>
      listeners.delete(listener),
    ),
  };
}

describe("device lists", () => {
  it("are read on mount and on every plug, without asking for a camera", () => {
    const media = fakeDevices();
    const list = vi.fn();
    const stop = watchMediaDevices(media as unknown as MediaDevices, list);
    expect(list).toHaveBeenCalledOnce();
    for (const listener of media.listeners) listener();
    expect(list).toHaveBeenCalledTimes(2);
    stop();
    expect(media.listeners.size).toBe(0);
    expect(media.getUserMedia).not.toHaveBeenCalled();
  });

  it("ask for one kind's permission only", async () => {
    const media = fakeDevices();
    await revealDeviceNames(media as unknown as MediaDevices, "audio");
    expect(media.getUserMedia).toHaveBeenCalledWith({ audio: true });
    await revealDeviceNames(media as unknown as MediaDevices, "video");
    expect(media.getUserMedia).toHaveBeenLastCalledWith({ video: true });
    expect(media.getUserMedia).toHaveBeenCalledTimes(2);
  });

  it("say when the permission was refused", async () => {
    const media = fakeDevices();
    media.getUserMedia.mockRejectedValueOnce(new Error("NotAllowedError"));
    expect(
      await revealDeviceNames(media as unknown as MediaDevices, "audio"),
    ).toBe(false);
  });
});
