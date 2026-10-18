import { afterEach, describe, expect, it } from "vitest";
import {
  markDesktopRoot,
  setNativeBridgeForTests,
  type NativeBridge,
} from "./bridge.ts";

const fakeRoot = () => ({ dataset: {} }) as unknown as HTMLElement;

const bridge: NativeBridge = {
  invoke: () => Promise.reject(new Error("unused")),
  channel: () => Promise.reject(new Error("unused")),
};

afterEach(() => setNativeBridgeForTests(undefined));

describe("markDesktopRoot", () => {
  it("marks the root in the desktop app", () => {
    setNativeBridgeForTests(bridge);
    const root = fakeRoot();
    markDesktopRoot(root);
    expect(root.dataset.desktop).toBe("");
  });

  it("leaves a browser's root alone", () => {
    setNativeBridgeForTests(null);
    const root = fakeRoot();
    markDesktopRoot(root);
    expect(root.dataset.desktop).toBeUndefined();
  });
});
