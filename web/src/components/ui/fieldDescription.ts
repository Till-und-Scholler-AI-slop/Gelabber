/** The `aria-describedby` of a field: its error, else its hint, plus `extra`. */
export function fieldDescription(
  id: string,
  { error, hint, extra }: { error?: unknown; hint?: unknown; extra?: string },
): string | undefined {
  const own = error ? `${id}-error` : hint ? `${id}-hint` : undefined;
  return [own, extra].filter(Boolean).join(" ") || undefined;
}
