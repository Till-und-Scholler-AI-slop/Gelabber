import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { pageZoom } from "../../interface/zoom.ts";
import { TIP_GRACE_MS, TIP_WARM_MS, tipDelay } from "../tooltipTiming.ts";
import "./ui.css";

/** What the control spreads onto itself so its tooltip follows it. */
export type TooltipTrigger = {
  onPointerEnter: (event: PointerEvent<HTMLElement>) => void;
  onPointerLeave: () => void;
  onFocus: (event: FocusEvent<HTMLElement>) => void;
  onBlur: () => void;
};

// One tooltip at a time on the page: moving from one control to the next
// moves it there at once (tooltipTiming.ts).
let showing = 0;
let warmUntil = 0;

type Shown = {
  anchor: HTMLElement;
  host: Element;
  /** Keyboard focus asked for it. */
  keyboard: boolean;
};

/**
 * The tooltip of a control, in the look of the server rail's: the action
 * in bold, its shortcut or a detail muted beneath, above the control. A
 * pointer resting on it shows it after a moment, keyboard focus at once;
 * Esc puts it away, as does leaving. Shown for the focus, that is all the
 * Esc does (as in the server rail); under a resting pointer the Esc still
 * goes on to what it would do anyway, such as "als gelesen markieren".
 * Screen readers get the same from the control's name and
 * `aria-keyshortcuts`, so the tooltip itself is hidden from them.
 *
 * The control spreads `trigger`; `tooltip` goes next to it.
 */
export function useTooltip({
  label,
  detail,
  disabled = false,
}: {
  label: string;
  detail?: string | null;
  /** While the control's own popover or menu is open. */
  disabled?: boolean;
}): { trigger: TooltipTrigger; tooltip: ReactNode } {
  const [shown, setShown] = useState<Shown | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const tip = useRef<HTMLDivElement>(null);
  const visible = shown !== null && !disabled;
  const byKeyboard = shown?.keyboard ?? false;

  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEffect(() => {
    if (!visible) return;
    showing += 1;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (byKeyboard && !event.defaultPrevented) event.preventDefault();
      window.clearTimeout(timer.current);
      setShown(null);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      showing -= 1;
      warmUntil = Date.now() + TIP_WARM_MS;
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [visible, byKeyboard]);

  // Above the control, centred, kept inside the window; below it where
  // there is no room above.
  useLayoutEffect(() => {
    const element = tip.current;
    if (!visible || !element || !shown) return;
    const zoom = pageZoom(shown.anchor);
    const box = shown.anchor.getBoundingClientRect();
    const width = element.offsetWidth;
    const height = element.offsetHeight;
    const viewport = window.innerWidth / zoom;
    const middle = (box.left + box.right) / 2 / zoom;
    const left = Math.min(
      Math.max(8, middle - width / 2),
      Math.max(8, viewport - 8 - width),
    );
    const above = box.top / zoom - 6 - height;
    element.style.left = `${left}px`;
    element.style.top = `${above >= 8 ? above : box.bottom / zoom + 6}px`;
  });

  const show = (anchor: HTMLElement, keyboard: boolean) => {
    window.clearTimeout(timer.current);
    const fullscreen = document.fullscreenElement;
    // Inside a picture in fullscreen nothing else shows.
    const host = fullscreen?.contains(anchor) ? fullscreen : document.body;
    const delay = tipDelay({
      showing: showing > 0,
      warm: Date.now() < warmUntil,
      keyboard,
    });
    const next = { anchor, host, keyboard };
    if (delay === 0) setShown(next);
    else timer.current = window.setTimeout(() => setShown(next), delay);
  };
  const hide = (at: "once" | "soon") => {
    window.clearTimeout(timer.current);
    if (at === "once") setShown(null);
    else timer.current = window.setTimeout(() => setShown(null), TIP_GRACE_MS);
  };

  return {
    trigger: {
      onPointerEnter: (event) => {
        if (event.pointerType === "mouse") show(event.currentTarget, false);
      },
      onPointerLeave: () => hide("soon"),
      onFocus: (event) => {
        if (event.currentTarget.matches(":focus-visible"))
          show(event.currentTarget, true);
      },
      onBlur: () => hide("once"),
    },
    tooltip: visible
      ? createPortal(
          <div ref={tip} className="ui-tooltip" aria-hidden="true">
            <strong>{label}</strong>
            {detail ? <span>{detail}</span> : null}
          </div>,
          shown.host,
        )
      : null,
  };
}
