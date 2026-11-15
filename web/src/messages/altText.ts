// Alt texts of pictures not sent yet (#575), by the very File: the
// composer's draft, the send attempt and its retry hold the same objects.

/** The server's limit, in characters. */
export const ALT_TEXT_MAX = 1000;

const texts = new WeakMap<File, string>();

export function altTextOf(file: File): string | undefined {
  return texts.get(file);
}

/** Sets the file's alt text; blank removes it. */
export function setAltText(file: File, text: string): void {
  const trimmed = text.trim();
  if (trimmed) texts.set(file, trimmed);
  else texts.delete(file);
}

/** Control characters the server refuses (all but tab and line break),
 *  as `validateContent` checks message text. */
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g;

/** The text without the control characters a paste can bring along, so a
 *  save does not fail on what nobody can see. */
export function withoutControls(text: string): string {
  return text.replace(CONTROL, "");
}

/** Characters as the server counts them (code points, not UTF-16 units). */
export function altTextLength(text: string): number {
  return [...text.trim()].length;
}
