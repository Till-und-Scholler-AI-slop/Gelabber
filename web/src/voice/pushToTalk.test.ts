import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setNativeBridgeForTests, type NativeBridge } from "./native/bridge.ts";
import {
  clampReleaseDelay,
  configurePushToTalk,
  globalKeyNote,
  onPushToTalkGate,
  pushToTalkCode,
  pushToTalkKeyLabel,
  pushToTalkOpen,
  resetPushToTalkForTests,
  usePushToTalk,
} from "./pushToTalk.ts";

class FakeElement extends EventTarget {
  isContentEditable = false;
}
class FakeInput extends FakeElement {
  type = "text";
}
class FakeTextArea extends FakeElement {}

let page: EventTarget;
let gate: ReturnType<typeof vi.fn<() => void>>;

function press(type: "keydown" | "keyup", code: string, target?: EventTarget) {
  const event = Object.assign(new Event(type, { cancelable: true }), { code });
  if (target) Object.defineProperty(event, "target", { value: target });
  page.dispatchEvent(event);
  return event;
}

function mouse(type: "mousedown" | "mouseup", button: number) {
  const event = Object.assign(new Event(type, { cancelable: true }), {
    button,
  });
  page.dispatchEvent(event);
  return event;
}

type Sent = { command: string; args?: Record<string, unknown> };

function fakeNative(fail = false) {
  const sent: Sent[] = [];
  const channels: ((message: unknown) => void)[] = [];
  const bridge: NativeBridge = {
    async invoke<T>(command: string, args?: Record<string, unknown>) {
      sent.push({ command, args });
      if (fail && command === "ptt_bind") throw new Error("not allowed");
      return undefined as T;
    },
    async channel<T>(onMessage: (message: T) => void) {
      channels.push(onMessage as (message: unknown) => void);
      return { id: channels.length };
    },
  };
  setNativeBridgeForTests(bridge);
  return { sent, channels };
}

beforeEach(() => {
  page = new EventTarget();
  vi.stubGlobal("window", page);
  vi.stubGlobal(
    "document",
    Object.assign(new EventTarget(), { visibilityState: "visible" }),
  );
  vi.stubGlobal("HTMLElement", FakeElement);
  vi.stubGlobal("HTMLInputElement", FakeInput);
  vi.stubGlobal("HTMLTextAreaElement", FakeTextArea);
  setNativeBridgeForTests(null);
  gate = vi.fn<() => void>();
  onPushToTalkGate(gate);
});

afterEach(() => {
  resetPushToTalkForTests();
  setNativeBridgeForTests(undefined);
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("push-to-talk", () => {
  it("leaves the microphone alone without a binding", () => {
    expect(pushToTalkOpen()).toBe(true);
    press("keydown", "F13");
    expect(gate).not.toHaveBeenCalled();
  });

  it("opens while the key is held and closes after the release delay", () => {
    vi.useFakeTimers();
    configurePushToTalk({ key: "F13", releaseDelay: 150 });
    expect(pushToTalkOpen()).toBe(false);
    expect(gate).toHaveBeenCalledTimes(1);
    press("keydown", "F13");
    // A held key repeats its key-down.
    press("keydown", "F13");
    expect(pushToTalkOpen()).toBe(true);
    expect(gate).toHaveBeenCalledTimes(2);
    press("keyup", "F13");
    vi.advanceTimersByTime(149);
    expect(pushToTalkOpen()).toBe(true);
    // Pressed again within the delay: never closed in between.
    press("keydown", "F13");
    vi.advanceTimersByTime(500);
    expect(pushToTalkOpen()).toBe(true);
    press("keyup", "F13");
    vi.advanceTimersByTime(150);
    expect(pushToTalkOpen()).toBe(false);
    expect(gate).toHaveBeenCalledTimes(3);
    // Other keys do nothing.
    press("keydown", "KeyA");
    expect(pushToTalkOpen()).toBe(false);
  });

  it("does not take a letter typed into a text field, but a function key", () => {
    configurePushToTalk({ key: "KeyV", releaseDelay: 0 });
    press("keydown", "KeyV", new FakeTextArea());
    expect(pushToTalkOpen()).toBe(false);
    press("keydown", "KeyV", new FakeElement());
    expect(pushToTalkOpen()).toBe(true);
    press("keyup", "KeyV", new FakeTextArea());
    expect(pushToTalkOpen()).toBe(false);

    configurePushToTalk({ key: "F8", releaseDelay: 0 });
    press("keydown", "F8", new FakeInput());
    expect(pushToTalkOpen()).toBe(true);
  });

  it("keeps the bound key from clicking a focused button", () => {
    configurePushToTalk({ key: "Space", releaseDelay: 0 });
    const button = new FakeElement();
    expect(press("keydown", "Space", button).defaultPrevented).toBe(true);
    expect(press("keyup", "Space", button).defaultPrevented).toBe(true);
    // Typed into a field, Space stays a space.
    expect(press("keydown", "Space", new FakeInput()).defaultPrevented).toBe(
      false,
    );
    expect(press("keyup", "Space", new FakeInput()).defaultPrevented).toBe(
      false,
    );
    expect(pushToTalkOpen()).toBe(false);
    // Other keys keep their default.
    expect(press("keydown", "Tab", button).defaultPrevented).toBe(false);
  });

  it("ignores the app's global key while typing it into this page", async () => {
    const { channels } = fakeNative();
    const field = { focused: true, active: new FakeTextArea() as EventTarget };
    // A getter of its own: Object.assign would copy its first value.
    const document = Object.defineProperty(
      Object.assign(new EventTarget(), {
        visibilityState: "visible",
        hasFocus: () => field.focused,
      }),
      "activeElement",
      { get: () => field.active },
    );
    vi.stubGlobal("document", document);
    configurePushToTalk({ key: "KeyV", releaseDelay: 0 });
    await vi.waitFor(() => expect(channels).toHaveLength(1));
    const native = channels[0];
    native({ type: "press" });
    expect(pushToTalkOpen()).toBe(false);
    native({ type: "release" });
    // Another window in front: the key is the push-to-talk key again.
    field.focused = false;
    native({ type: "press" });
    expect(pushToTalkOpen()).toBe(true);
    native({ type: "release" });
    // In the window, away from a text field, too.
    field.focused = true;
    field.active = new FakeElement();
    native({ type: "press" });
    expect(pushToTalkOpen()).toBe(true);
  });

  it("takes a side mouse button and keeps it from navigating", () => {
    configurePushToTalk({ key: "Mouse4", releaseDelay: 0 });
    expect(mouse("mousedown", 0).defaultPrevented).toBe(false);
    expect(pushToTalkOpen()).toBe(false);
    expect(mouse("mousedown", 3).defaultPrevented).toBe(true);
    expect(pushToTalkOpen()).toBe(true);
    expect(mouse("mouseup", 3).defaultPrevented).toBe(true);
    expect(pushToTalkOpen()).toBe(false);
  });

  it("lets go when the window loses the focus", () => {
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    press("keydown", "F13");
    page.dispatchEvent(new Event("blur"));
    expect(pushToTalkOpen()).toBe(false);
  });

  it("keeps the microphone closed when no key is bound", () => {
    configurePushToTalk({ key: "", releaseDelay: 0 });
    expect(usePushToTalk.getState().active).toBe(true);
    expect(pushToTalkOpen()).toBe(false);
    configurePushToTalk(null);
    expect(pushToTalkOpen()).toBe(true);
    expect(usePushToTalk.getState().active).toBe(false);
    expect(gate).toHaveBeenCalledTimes(2);
  });

  it("changes the release delay without binding the key again", async () => {
    const { sent } = fakeNative();
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    configurePushToTalk({ key: "F13", releaseDelay: 300 });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0].command).toBe("ptt_bind");
  });

  it("binds the key globally in the desktop app and follows its events", async () => {
    const { sent, channels } = fakeNative();
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    expect(usePushToTalk.getState().global).toEqual({ state: "pending" });
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]).toMatchObject({
      command: "ptt_bind",
      args: { key: "F13" },
    });
    const native = channels[0];
    native({ type: "bound", trigger: "F13" });
    expect(usePushToTalk.getState().global).toEqual({
      state: "bound",
      trigger: "F13",
    });
    native({ type: "press" });
    expect(pushToTalkOpen()).toBe(true);
    // The window's own listener hears the same key: still one hold.
    press("keydown", "F13");
    press("keyup", "F13");
    expect(pushToTalkOpen()).toBe(true);
    native({ type: "release" });
    expect(pushToTalkOpen()).toBe(false);

    configurePushToTalk(null);
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(sent[1].command).toBe("ptt_unbind");
    // The old binding's channel no longer counts.
    configurePushToTalk({ key: "F14", releaseDelay: 0 });
    native({ type: "press" });
    expect(pushToTalkOpen()).toBe(false);
    await vi.waitFor(() => expect(sent).toHaveLength(3));
    expect(sent[2]).toMatchObject({
      command: "ptt_bind",
      args: { key: "F14" },
    });
  });

  it("falls back to the window when the app refuses the global key", async () => {
    fakeNative(true);
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    await vi.waitFor(() =>
      expect(usePushToTalk.getState().global).toEqual({
        state: "failed",
        reason: "not allowed",
      }),
    );
    press("keydown", "F13");
    expect(pushToTalkOpen()).toBe(true);
  });

  it("names keys for the settings", () => {
    expect(pushToTalkKeyLabel("KeyV")).toBe("V");
    expect(pushToTalkKeyLabel("Digit4")).toBe("4");
    expect(pushToTalkKeyLabel("ControlLeft")).toBe("Control links");
    expect(pushToTalkKeyLabel("Numpad0")).toBe("Num 0");
    expect(pushToTalkKeyLabel("Mouse5")).toBe("Maustaste 5");
    expect(pushToTalkCode({ code: "Escape" })).toBeNull();
    expect(pushToTalkCode({ code: "F13" })).toBe("F13");
    expect(clampReleaseDelay(5000)).toBe(1000);
    expect(clampReleaseDelay(-1)).toBe(0);
    expect(clampReleaseDelay("x")).toBe(200);
  });

  it("explains the global key per system", () => {
    expect(globalKeyNote({ state: "off" }, true)).not.toContain("Hyprland");
    expect(globalKeyNote({ state: "off" }, false)).toContain("hyprland.conf");
    expect(globalKeyNote({ state: "bound", trigger: "Strg+F13" }, false)).toBe(
      "Global aktiv: Strg+F13.",
    );
    expect(
      globalKeyNote({ state: "failed", reason: "Abgebrochen." }, false),
    ).toContain("nur, solange Gelabber im Vordergrund ist: Abgebrochen.");
  });
});
