import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  callShortcut,
  callShortcutKeys,
  callShortcutLabel,
  clampReleaseDelay,
  configurePushToTalk,
  holdToTalk,
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
  gate = vi.fn<() => void>();
  onPushToTalkGate(gate);
});

afterEach(() => {
  resetPushToTalkForTests();
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

  it("changes the release delay without binding the key again", () => {
    vi.useFakeTimers();
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    configurePushToTalk({ key: "F13", releaseDelay: 300 });
    expect(gate).toHaveBeenCalledTimes(1);
    press("keydown", "F13");
    press("keyup", "F13");
    vi.advanceTimersByTime(299);
    expect(pushToTalkOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(pushToTalkOpen()).toBe(false);
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

  it("opens the microphone while the touch button is held, with or without a key", () => {
    configurePushToTalk({ key: "", releaseDelay: 0 });
    expect(pushToTalkOpen()).toBe(false);
    expect(gate).toHaveBeenCalledTimes(1);
    holdToTalk(true);
    expect(pushToTalkOpen()).toBe(true);
    expect(gate).toHaveBeenCalledTimes(2);
    holdToTalk(false);
    expect(pushToTalkOpen()).toBe(false);
    // The key and the button together: released only when both are.
    configurePushToTalk({ key: "F13", releaseDelay: 0 });
    press("keydown", "F13");
    holdToTalk(true);
    press("keyup", "F13");
    expect(pushToTalkOpen()).toBe(true);
    holdToTalk(false);
    expect(pushToTalkOpen()).toBe(false);
    // Outside a call it does nothing.
    configurePushToTalk(null);
    holdToTalk(true);
    expect(usePushToTalk.getState().held).toBe(false);
  });
});

describe("call shortcuts", () => {
  const key = (code: string, extra: Partial<KeyboardEvent> = {}) => ({
    code,
    ctrlKey: true,
    metaKey: false,
    shiftKey: true,
    altKey: false,
    repeat: false,
    ...extra,
  });

  it("mutes with Strg+Umschalt+M and deafens with Strg+Umschalt+D", () => {
    expect(callShortcut(key("KeyM"), null)).toBe("mute");
    expect(callShortcut(key("KeyD"), null)).toBe("deafen");
    expect(
      callShortcut(key("KeyM", { ctrlKey: false, metaKey: true }), null),
    ).toBe("mute");
  });

  it("leaves every other chord alone", () => {
    expect(callShortcut(key("KeyM", { shiftKey: false }), null)).toBeNull();
    expect(callShortcut(key("KeyM", { ctrlKey: false }), null)).toBeNull();
    expect(callShortcut(key("KeyM", { altKey: true }), null)).toBeNull();
    expect(callShortcut(key("KeyM", { metaKey: true }), null)).toBeNull();
    expect(callShortcut(key("KeyM", { repeat: true }), null)).toBeNull();
    expect(
      callShortcut({ ...key("KeyD"), isComposing: true }, null),
    ).toBeNull();
    expect(callShortcut(key("KeyK"), null)).toBeNull();
  });

  it("skips a combination whose letter is the push-to-talk key", () => {
    expect(callShortcut(key("KeyM"), "KeyM")).toBeNull();
    expect(callShortcut(key("KeyD"), "KeyM")).toBe("deafen");
    expect(callShortcut(key("KeyM"), "ControlLeft")).toBe("mute");
  });

  it("writes the shortcut the way the platform does", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" });
    expect(callShortcutLabel("mute")).toBe("⌘⇧M");
    expect(callShortcutKeys("mute")).toBe("Meta+Shift+M");
    vi.stubGlobal("navigator", { platform: "Linux x86_64", userAgent: "" });
    expect(callShortcutLabel("deafen")).toBe("Strg+Umschalt+D");
    expect(callShortcutKeys("deafen")).toBe("Control+Shift+D");
  });
});
