// Push-to-talk (issue #165): in this mode the microphone sends only while
// the bound key is held, and for the release delay after. A browser tab
// hears the key only while it has the focus; the desktop app also hears it
// with the window in the background (desktop/app/src/ptt.rs). The session
// configures this while it is in a call, asks `pushToTalkOpen()` whether the
// microphone may send and is told through `onPushToTalkGate` when that
// changes. Mute and deafen stay the session's own and win over the key.

import { create } from "zustand";
import { isDesktopApp, nativeBridge } from "./native/bridge.ts";

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

/** The side buttons of a mouse, as the desktop app names them. */
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

/** The global key of the desktop app. */
export type GlobalKey =
  | { state: "off" }
  | { state: "pending" }
  | { state: "bound"; trigger: string | null }
  | { state: "failed"; reason: string };

const HYPRLAND_BIND = "bind = , <Taste>, global, gelabber-desktop:push-to-talk";

export function onWindows(): boolean {
  return (
    typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent)
  );
}

/** What the desktop app does with the key, in a call and before one. */
export function globalKeyNote(global: GlobalKey, windows: boolean): string {
  const linuxBinding = `Unter Linux bestimmt der Desktop die Taste: KDE und GNOME fragen beim ersten Mal nach, unter Hyprland gehört „${HYPRLAND_BIND}“ in die hyprland.conf.`;
  switch (global.state) {
    case "off":
      return windows
        ? "Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist."
        : `Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist. ${linuxBinding}`;
    case "pending":
      return "Die globale Taste wird eingerichtet …";
    case "bound":
      if (global.trigger) return `Global aktiv: ${global.trigger}.`;
      return windows
        ? "Global aktiv, auch wenn ein anderes Fenster vorne ist."
        : `Global aktiv. ${linuxBinding}`;
    case "failed":
      return `Die Taste wirkt nur, solange Gelabber im Vordergrund ist: ${global.reason}`;
  }
}

type PushToTalkState = {
  /** Push-to-talk is on in a running call. */
  active: boolean;
  /** The key is down. */
  held: boolean;
  /** The microphone may send: the key is down or was released within the
   * delay. */
  open: boolean;
  global: GlobalKey;
};

const off: PushToTalkState = {
  active: false,
  held: false,
  open: false,
  global: { state: "off" },
};

export const usePushToTalk = create<PushToTalkState>(() => ({ ...off }));

export type PushToTalkBinding = { key: string; releaseDelay: number };

type NativeEvent =
  | { type: "bound"; trigger: string | null }
  | { type: "failed"; reason: string }
  | { type: "press" }
  | { type: "release" };

let binding: PushToTalkBinding | null = null;
let gateListener: (() => void) | null = null;
let pageHeld = false;
let nativeHeld = false;
let touchHeld = false;
let releaseTimer: ReturnType<typeof setTimeout> | null = null;
/** Moves with every binding; events of an older one are dropped. */
let generation = 0;
let detachPage: (() => void) | null = null;
/** Native binds and unbinds in the order they were asked for. */
let nativeQueue: Promise<void> = Promise.resolve();
let nativeBound = false;

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
  generation += 1;
  detachPage?.();
  detachPage = null;
  if (releaseTimer !== null) clearTimeout(releaseTimer);
  releaseTimer = null;
  pageHeld = false;
  nativeHeld = false;
  touchHeld = false;
  binding = null;
  if (nativeBound) {
    nativeBound = false;
    queueNative(() => nativeBridge().invoke("ptt_unbind"));
  }
  usePushToTalk.setState({ ...off });
}

function begin(next: PushToTalkBinding): void {
  binding = next;
  usePushToTalk.setState({ ...off, active: true });
  if (!next.key) return;
  detachPage = listenInPage(next.key);
  if (isDesktopApp()) bindNative(next.key, generation);
}

function queueNative(job: () => Promise<unknown>): void {
  nativeQueue = nativeQueue.then(job).then(
    () => undefined,
    () => undefined,
  );
}

function bindNative(key: string, bound: number): void {
  usePushToTalk.setState({ global: { state: "pending" } });
  nativeBound = true;
  queueNative(async () => {
    if (bound !== generation) return;
    try {
      const events = await nativeBridge().channel<NativeEvent>((event) => {
        if (bound === generation) onNativeEvent(event);
      });
      await nativeBridge().invoke("ptt_bind", { key, events });
    } catch (error) {
      if (bound !== generation) return;
      usePushToTalk.setState({
        global: {
          state: "failed",
          reason: error instanceof Error ? error.message : String(error),
        },
      });
    }
  });
}

function onNativeEvent(event: NativeEvent): void {
  switch (event.type) {
    case "bound":
      usePushToTalk.setState({
        global: { state: "bound", trigger: event.trigger ?? null },
      });
      return;
    case "failed":
      usePushToTalk.setState({
        global: { state: "failed", reason: event.reason },
      });
      setHeld("native", false);
      return;
    case "press":
      // The app hears the key in every window, its own too: typed into a
      // field of this page, it is text, as for the page's own listener.
      if (binding && typingInFocusedPage(binding.key)) return;
      setHeld("native", true);
      return;
    case "release":
      setHeld("native", false);
      return;
  }
}

function setHeld(source: "page" | "native" | "touch", down: boolean): void {
  if (!binding) return;
  if (source === "page") pageHeld = down;
  else if (source === "native") nativeHeld = down;
  else touchHeld = down;
  const held = pageHeld || nativeHeld || touchHeld;
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

function typingInFocusedPage(key: string): boolean {
  if (typeof document === "undefined") return false;
  return (
    (document.hasFocus?.() ?? false) && typingHere(key, document.activeElement)
  );
}

/** The key while this page has the focus, in a browser and in the app. */
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
  nativeQueue = Promise.resolve();
}
