// Which emoji this device can draw. The picker offers Unicode 18 emoji, but
// an older system emoji font draws the newer ones as an empty box (#375:
// U+1FAE8). The picker leaves out what the fonts here cannot draw.
// Measured once per browser and emoji list, then remembered.

import { useEffect, useState } from "react";

const STORAGE_KEY = "gelabber.emoji-missing";
const SIZE = 24;
/** Emoji tested per slice, so the check never blocks a frame for long. */
const SLICE = 150;
const NONE: ReadonlySet<string> = new Set();

/** Only emoji from the supplementary planes or joined sequences can be too
 *  new; the older symbols (©, ☺, ↔ …) draw on every system, as text or as
 *  a picture. */
export function needsCheck(emoji: string): boolean {
  return emoji.includes("‍") || (emoji.codePointAt(0) ?? 0) >= 0x1f000;
}

/** A colour emoji keeps its own colours whatever the fill; a box or a
 *  plain glyph takes the fill. Drawn in two fills, a picture leaves the
 *  same pixels and some ink. */
export function samePicture(
  red: Uint8ClampedArray,
  blue: Uint8ClampedArray,
): boolean {
  let ink = false;
  for (let i = 0; i < red.length; i += 4) {
    if (red[i + 3] !== 0) ink = true;
    if (
      red[i] !== blue[i] ||
      red[i + 1] !== blue[i + 1] ||
      red[i + 2] !== blue[i + 2] ||
      red[i + 3] !== blue[i + 3]
    )
      return false;
  }
  return ink;
}

type Probe = (emoji: string) => boolean;

/** Null without a canvas or without any colour emoji font: then nothing is
 *  hidden, rather than everything. */
function createProbe(): Probe | null {
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = SIZE * 2;
  canvas.height = SIZE;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return null;
  context.font = `${SIZE - 4}px sans-serif`;
  context.textBaseline = "top";
  const pixels = (emoji: string, fill: string) => {
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = fill;
    context.fillText(emoji, 0, 0);
    return context.getImageData(0, 0, SIZE, SIZE).data;
  };
  const single = context.measureText("😀").width;
  const draws: Probe = (emoji) =>
    // A joined sequence the font does not know falls apart into its parts.
    context.measureText(emoji).width < single * 1.5 &&
    samePicture(pixels(emoji, "#ff0000"), pixels(emoji, "#0000ff"));
  return draws("😀") ? draws : null;
}

function storageKey(version: string): string {
  return `${version} ${typeof navigator === "undefined" ? "" : navigator.userAgent}`;
}

export function readMissing(version: string): ReadonlySet<string> | null {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as {
      key?: unknown;
      missing?: unknown;
    } | null;
    if (saved?.key !== storageKey(version) || !Array.isArray(saved.missing))
      return null;
    return new Set(saved.missing.filter((e) => typeof e === "string"));
  } catch {
    return null;
  }
}

function saveMissing(version: string, missing: ReadonlySet<string>): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ key: storageKey(version), missing: [...missing] }),
    );
  } catch {
    // Private mode or full storage: measured again next time.
  }
}

/** The emoji from `list` this device cannot draw. Empty until measured; the
 *  first measurement runs in slices after the picker opened. */
export function useMissingEmoji(
  list: readonly string[],
  version: string,
): ReadonlySet<string> {
  const [missing, setMissing] = useState(() => readMissing(version) ?? NONE);
  useEffect(() => {
    if (readMissing(version)) return;
    const probe = createProbe();
    if (!probe) return;
    const found = new Set<string>();
    let next = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const slice = () => {
      const end = Math.min(next + SLICE, list.length);
      for (; next < end; next++) {
        const emoji = list[next];
        if (needsCheck(emoji) && !probe(emoji)) found.add(emoji);
      }
      if (next < list.length) {
        timer = setTimeout(slice, 0);
        return;
      }
      saveMissing(version, found);
      setMissing(found);
    };
    timer = setTimeout(slice, 0);
    return () => clearTimeout(timer);
  }, [list, version]);
  return missing;
}
