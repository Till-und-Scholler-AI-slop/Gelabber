// Push-to-talk (issue #165): in this mode the microphone sends only while
// the bound key is held, and for the release delay after. A browser tab
// hears the key only while it has the focus. The session configures this while it is in a call, asks `pushToTalkOpen()` whether the
// microphone may send and is told through `onPushToTalkGate` when that
// changes. Mute and deafen stay the session's own and win over the key.

import { create } from "zustand";

export type VoiceInputMode = "activity" | "ptt";

export function asVoiceInputMode(value: unknown): VoiceInputMode {
  return value === "ptt" ? "ptt" : "activity";
}

export const PTT_RELEASE_DELAY_MAX = 1000;
export const PTT_RELEASE_DELAY_DEFAULT = 200;

export function clampReleaseDelay(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    return PTT_RELEASE_DELAY_DEFAULT;
  return Math.round(Math.min(PTT_RELEASE_DELAY_MAX, Math.max(0, value)));
}

/** The side buttons of a mouse, as the native app names them too. */
const MOUSE_BUTTONS: Record<string, number> = { Mouse4: 3, Mouse5: 4 };

/** The key as the settings show it, when no label was kept. */
export function pushToTalkKeyLabel(code: string): string {
  if (code === "Mouse4") return "Maustaste 4";
  if (code === "Mouse5") return "Maustaste 5";
  if (code === "Space") return "Leertaste";
  return code
    .replace(/^Key(?=[A-Z]$)/, "")
    .replace(/^Digit(?=\d$)/, "")
    .replace(/^Numpad/, "Num ")
    .replace(/Left$/, " links")
    .replace(/Right$/, " rechts");
}

/** A key event's binding, or null for keys that cannot be one. */
export function pushToTalkCode(event: { code: string }): string | null {
  const code = event.code;
  if (!code || code === "Escape" || code === "Enter" || code === "Backspace")
    return null;
  return code;
}

export function pushToTalkMouseCode(button: number): string | null {
  return (
    Object.entries(MOUSE_BUTTONS).find(([, value]) => value === button)?.[0] ??
    null
  );
}

type PushToTalkState = {
  /** Push-to-talk is on in a running call. */
  active: boolean;
  /** The key is down. */
  held: boolean;
  /** The microphone may send: the key is down or was released within the
   * delay. */
  open: boolean;
};

const off: PushToTalkState = {
  active: false,
  held: false,
  open: false,
};

export const usePushToTalk = create<PushToTalkState>(() => ({ ...off }));

export type PushToTalkBinding = { key: string; releaseDelay: number };

let binding: PushToTalkBinding | null = null;
let gateListener: (() => void) | null = null;
let pageHeld = false;
let touchHeld = false;
let releaseTimer: ReturnType<typeof setTimeout> | null = null;
let detachPage: (() => void) | null = null;

/** Whether the microphone may send as far as push-to-talk goes. */
export function pushToTalkOpen(): boolean {
  return binding === null || usePushToTalk.getState().open;
}

export function onPushToTalkGate(listener: (() => void) | null): void {
  gateListener = listener;
}

/** The binding while in a call with push-to-talk on, else null. A key of
 * "" is push-to-talk without a key: the microphone stays closed. */
export function configurePushToTalk(next: PushToTalkBinding | null): void {
  if (binding && next && binding.key === next.key) {
    binding = next;
    return;
  }
  if (binding === null && next === null) return;
  const wasOpen = pushToTalkOpen();
  end();
  if (next) begin(next);
  if (pushToTalkOpen() !== wasOpen) gateListener?.();
}

function end(): void {
  detachPage?.();
  detachPage = null;
  if (releaseTimer !== null) clearTimeout(releaseTimer);
  releaseTimer = null;
  pageHeld = false;
  touchHeld = false;
  binding = null;
  usePushToTalk.setState({ ...off });
}

function begin(next: PushToTalkBinding): void {
  binding = next;
  usePushToTalk.setState({ ...off, active: true });
  if (!next.key) return;
  detachPage = listenInPage(next.key);
}

function setHeld(source: "page" | "touch", down: boolean): void {
  if (!binding) return;
  if (source === "page") pageHeld = down;
  else touchHeld = down;
  const held = pageHeld || touchHeld;
  const state = usePushToTalk.getState();
  if (held === state.held) return;
  if (releaseTimer !== null) clearTimeout(releaseTimer);
  releaseTimer = null;
  if (held) {
    usePushToTalk.setState({ held: true, open: true });
    if (!state.open) gateListener?.();
    return;
  }
  usePushToTalk.setState({ held: false });
  if (binding.releaseDelay > 0)
    releaseTimer = setTimeout(close, binding.releaseDelay);
  else close();
}

function close(): void {
  releaseTimer = null;
  const state = usePushToTalk.getState();
  if (state.held || !state.open) return;
  usePushToTalk.setState({ open: false });
  gateListener?.();
}

/** The hold-to-talk button of a touch screen, held like the key. It works
 * without a key, too: a phone has none to bind. */
export function holdToTalk(down: boolean): void {
  setHeld("touch", down);
}

export type CallShortcut = "mute" | "deafen";

const CALL_SHORTCUT_KEYS: Record<string, CallShortcut> = {
  KeyM: "mute",
  KeyD: "deafen",
};

/** Strg+Umschalt+M mutes and Strg+Umschalt+D deafens (⌘ instead of Strg on
 * a Mac), in text fields too: the combination types nothing. A push-to-talk
 * key on the same letter keeps it, so one press never does both. */
export function callShortcut(
  event: Pick<
    KeyboardEvent,
    "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey" | "repeat"
  > & { isComposing?: boolean },
  pttKey: string | null,
): CallShortcut | null {
  if (!event.shiftKey || event.altKey || event.repeat || event.isComposing)
    return null;
  // Strg or ⌘, not both: AltGr and other chords stay the system's.
  if (event.ctrlKey === event.metaKey) return null;
  const action = CALL_SHORTCUT_KEYS[event.code];
  if (!action || event.code === pttKey) return null;
  return action;
}

function applePlatform(): boolean {
  return (
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent || "")
  );
}

/** How the tooltips write the shortcut: "Strg+Umschalt+M" or "⌘+Umschalt+M". */
export function callShortcutLabel(action: CallShortcut): string {
  const key = action === "mute" ? "M" : "D";
  return `${applePlatform() ? "⌘" : "Strg"}+Umschalt+${key}`;
}

/** The same shortcut for `aria-keyshortcuts`: "Control+Shift+M". */
export function callShortcutKeys(action: CallShortcut): string {
  const key = action === "mute" ? "M" : "D";
  return `${applePlatform() ? "Meta" : "Control"}+Shift+${key}`;
}

/** Keys that write nothing into a text field; only these count while the
 * user types. */
function writesNothing(code: string): boolean {
  return (
    /^(F\d{1,2}|Shift|Control|Alt|Meta|Mouse)/.test(code) ||
    ["CapsLock", "Pause", "ScrollLock", "ContextMenu", "Insert"].includes(code)
  );
}

function typingInto(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement))
    return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLInputElement &&
      !["checkbox", "radio", "range", "button", "submit"].includes(target.type))
  );
}

/** A key that writes into the field it goes to is the user's text there,
 * not the push-to-talk key. */
function typingHere(key: string, target: EventTarget | null): boolean {
  return typingInto(target) && !writesNothing(key);
}

/** The key while this page has the focus. */
function listenInPage(key: string): () => void {
  if (typeof window === "undefined") return () => {};
  const button = MOUSE_BUTTONS[key];
  // The bound key does nothing else: Space would otherwise click a focused
  // button on every release, Tab move the focus, arrows scroll.
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.code !== key || typingHere(key, event.target)) return;
    event.preventDefault();
    setHeld("page", true);
  };
  const onKeyUp = (event: KeyboardEvent) => {
    if (event.code !== key) return;
    if (!typingHere(key, event.target)) event.preventDefault();
    setHeld("page", false);
  };
  const onMouse = (down: boolean) => (event: MouseEvent) => {
    if (button === undefined || event.button !== button) return;
    // The side buttons would otherwise go back and forward in history.
    event.preventDefault();
    setHeld("page", down);
  };
  const onMouseDown = onMouse(true);
  const onMouseUp = onMouse(false);
  const onBlur = () => setHeld("page", false);
  const onVisibility = () => {
    if (document.visibilityState === "hidden") setHeld("page", false);
  };
  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("mousedown", onMouseDown, true);
  window.addEventListener("mouseup", onMouseUp, true);
  window.addEventListener("blur", onBlur);
  document.addEventListener("visibilitychange", onVisibility);
  return () => {
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("mousedown", onMouseDown, true);
    window.removeEventListener("mouseup", onMouseUp, true);
    window.removeEventListener("blur", onBlur);
    document.removeEventListener("visibilitychange", onVisibility);
  };
}

/** Tests: back to no binding, without telling the session. */
export function resetPushToTalkForTests(): void {
  gateListener = null;
  end();
}
