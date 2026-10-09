// On phones the composer is as tall as its draft: chat.css asks for that with
// field-sizing. A browser without it (iOS before 26.2) gets the height from
// here instead, under the same max-height.

export type DraftHeight = {
  /** Measures again; for a changed draft, before the browser paints it. */
  fit: () => void;
};

/** Sizes `field` by its draft where the stylesheet cannot, null where it can.
 * `form` is the composer around the field, next to the message list. */
export function sizeByDraft(
  target: Window & typeof globalThis,
  field: HTMLTextAreaElement,
  form: HTMLElement,
): DraftHeight | null {
  if (target.CSS.supports("field-sizing", "content")) return null;
  // The stylesheet's own condition, asked each time like a media query.
  const touch = target.matchMedia("(pointer: coarse)");

  const fit = () => {
    if (!touch.matches) return;
    // The field collapses to one line while it is measured. The form keeps
    // its height meanwhile: the message list would otherwise grow for that
    // moment, be clamped away from its end by the browser and take the new
    // position for the user scrolling up.
    const held = form.style.minHeight;
    form.style.minHeight = `${Math.ceil(form.getBoundingClientRect().height)}px`;
    field.style.height = "auto";
    field.style.height = `${field.scrollHeight}px`;
    form.style.minHeight = held;
  };

  return { fit };
}
