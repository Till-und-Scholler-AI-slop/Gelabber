import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";

import { useReducedMotion } from "../../interface/useReducedMotion.ts";
import {
  exitFinished,
  mergePresence,
  sameValues,
  type PresenceEntry,
  type PresenceKey,
  type PresenceState,
} from "./presence.ts";

export type Presence = {
  /** Render the surface: open, or still leaving. */
  mounted: boolean;
  /** For its `data-state`; "closed" while it leaves. */
  state: PresenceState;
};

/**
 * Keeps a surface mounted after `open` turns false until `element` has
 * played its exit (CSS on `[data-state="closed"]`). With reduced motion it
 * unmounts at once. Opened again while leaving, it simply turns back.
 */
export function usePresence(
  open: boolean,
  element: RefObject<Element | null>,
): Presence {
  const reduced = useReducedMotion();
  const [wasOpen, setWasOpen] = useState(open);
  const [leaving, setLeaving] = useState(false);
  if (open !== wasOpen) {
    setWasOpen(open);
    setLeaving(!open && !reduced);
  }
  useLayoutEffect(() => {
    if (!leaving) return;
    let current = true;
    void exitFinished(element.current).then(() => {
      if (current) setLeaving(false);
    });
    return () => {
      current = false;
    };
  }, [leaving, element]);
  return { mounted: open || leaving, state: open ? "open" : "closed" };
}

/**
 * `values` while open; while closing, the ones of the last open render. A
 * leaving dialog keeps its title and buttons even when its owner already
 * cleared what they came from.
 */
export function useLastOpen<T extends readonly unknown[]>(
  open: boolean,
  values: T,
): T {
  const [kept, setKept] = useState(values);
  if (open && !sameValues(kept, values)) setKept(values);
  return open ? values : kept;
}

/**
 * A list whose entries leave the way a single surface does: one removed
 * from `items` stays, closed and in its place, until its element has played
 * the exit. Give each rendered entry `ref={presence.ref(entry.key)}`.
 */
export function usePresenceList<T>(
  items: readonly T[],
  keyOf: (item: T) => PresenceKey,
): {
  entries: PresenceEntry<T>[];
  ref: (key: PresenceKey) => (element: Element | null) => void;
} {
  const reduced = useReducedMotion();
  const [list, setList] = useState(() => ({
    items,
    entries: mergePresence([], items, keyOf, false),
  }));
  let entries = list.entries;
  if (list.items !== items) {
    entries = mergePresence(list.entries, items, keyOf, !reduced);
    setList({ items, entries });
  }
  const elements = useRef(new Map<PresenceKey, Element>());
  const waiting = useRef(new Set<PresenceKey>());
  useLayoutEffect(() => {
    const keys = new Set(entries.map((entry) => entry.key));
    for (const key of elements.current.keys())
      if (!keys.has(key)) elements.current.delete(key);
    for (const entry of entries) {
      if (entry.state !== "closed" || waiting.current.has(entry.key)) continue;
      const key = entry.key;
      waiting.current.add(key);
      void exitFinished(elements.current.get(key) ?? null).then(() => {
        waiting.current.delete(key);
        setList((current) => ({
          ...current,
          entries: current.entries.filter(
            (candidate) =>
              candidate.key !== key || candidate.state !== "closed",
          ),
        }));
      });
    }
  }, [entries]);
  const ref = useCallback(
    (key: PresenceKey) => (element: Element | null) => {
      if (element) elements.current.set(key, element);
    },
    [],
  );
  return { entries, ref };
}
