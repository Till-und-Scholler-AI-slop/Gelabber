// The composer is as tall as its draft: chat.css asks for that with
// field-sizing. A browser without it (Firefox, Safari before 26.2) gets the
// height from here instead, under the same max-height. With a mouse that
// limit is half the conversation (GEL-143), from `capByPane`.

/** The field's limit on phones, and the least it is elsewhere (chat.css). */
export const COMPOSER_MIN_CAP = 160;

/** Hands chat.css the field's limit beside a mouse: half the conversation
 * `pane`, at least `COMPOSER_MIN_CAP`. */
export function capByPane(form: HTMLElement, pane: HTMLElement): void {
  const cap = Math.max(COMPOSER_MIN_CAP, Math.floor(pane.clientHeight / 2));
  form.style.setProperty("--lr-composer-cap", `${cap}px`);
}

export type DraftHeight = {
  /** Measures again; for a changed draft, before the browser paints it. */
  fit: () => void;
  stop: () => void;
};

/** Sizes `field` by its draft where the stylesheet cannot, null where it can.
 * `form` is the composer around the field, next to the message list. */
export function sizeByDraft(
  target: Window & typeof globalThis,
  field: HTMLTextAreaElement,
  form: HTMLElement,
): DraftHeight | null {
  if (target.CSS.supports("field-sizing", "content")) return null;
  let width = 0;
  let frame = 0;

  const fit = () => {
    // The field collapses to one line while it is measured. The form keeps
    // its height meanwhile: the message list would otherwise grow for that
    // moment, be clamped away from its end by the browser and take the new
    // position for the user scrolling up.
    const held = form.style.minHeight;
    form.style.minHeight = `${Math.ceil(form.getBoundingClientRect().height)}px`;
    field.style.height = "auto";
    const height = field.scrollHeight;
    width = field.offsetWidth;
    field.style.height = `${height}px`;
    form.style.minHeight = held;
  };

  // The same draft wraps differently at another width (phone rotated).
  const observer = new target.ResizeObserver(() => {
    const next = field.offsetWidth;
    // No width: hidden, e.g. behind the search on a short screen.
    if (next === 0 || next === width) return;
    target.cancelAnimationFrame(frame);
    // In the next frame: a new height from inside the observer would change
    // the sizes it and the pane's other observers have just reported, which
    // the browser reports as a ResizeObserver loop.
    frame = target.requestAnimationFrame(fit);
  });
  observer.observe(field);

  return {
    fit,
    stop() {
      observer.disconnect();
      target.cancelAnimationFrame(frame);
    },
  };
}
