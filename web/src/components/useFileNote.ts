// The composer's note naming the files left out (GEL-8). It must not stay
// for good: the composer closes it on the next edit, on Escape, with its ×
// and on a send, and it goes by itself after `FILE_NOTE_MS`. The timer
// waits while the pointer is on the note or its × has focus, and starts
// over once both are gone.

import { useCallback, useEffect, useState } from "react";

export const FILE_NOTE_MS = 8_000;

// A new object for every note, so the same text again restarts the timer.
type Note = { text: string };

export function useFileNote() {
  const [note, setNote] = useState<Note | null>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const held = hovered || focused;

  /** Shows `text`, or closes the note for `null`. The same text again
   *  (the same file picked twice) gives it its full time again. */
  const show = useCallback((text: string | null) => {
    setHovered(false);
    setFocused(false);
    setNote(text === null ? null : { text });
  }, []);
  const close = useCallback(() => show(null), [show]);

  useEffect(() => {
    if (!note || held) return;
    const timer = setTimeout(() => setNote(null), FILE_NOTE_MS);
    return () => clearTimeout(timer);
  }, [note, held]);

  return {
    text: note?.text ?? null,
    show,
    close,
    /** Spread on the note: hover and focus hold its timer. */
    holdProps: {
      onPointerEnter: () => setHovered(true),
      onPointerLeave: () => setHovered(false),
      onFocus: () => setFocused(true),
      onBlur: () => setFocused(false),
    },
  };
}
