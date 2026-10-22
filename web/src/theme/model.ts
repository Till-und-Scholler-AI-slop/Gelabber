import { randomUuid } from "../lib/uuid.ts";

export const COLOR_KEYS = [
  "background",
  "panel",
  "surface",
  "rail",
  "text",
  "muted",
  "accent",
  "border",
] as const;
export type ThemeColors = Record<(typeof COLOR_KEYS)[number], string>;
export type ThemeStyle = "clear" | "soft" | "terminal";
export type ThemeDefinition = {
  version: 1;
  id: string;
  name: string;
  mode: "light" | "dark";
  style: ThemeStyle;
  colors: ThemeColors;
};
export type ThemeDocument = {
  version: 1;
  revision: number;
  active: string;
  customThemes: ThemeDefinition[];
};
export const MAX_THEMES = 50;
export const MAX_IMPORT_BYTES = 64 * 1024;
const object = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
const keysAre = (v: Record<string, unknown>, keys: readonly string[]) =>
  Object.keys(v).length === keys.length &&
  Object.keys(v).every((k) => keys.includes(k));
export function isTheme(v: unknown): v is ThemeDefinition {
  if (
    !object(v) ||
    !keysAre(v, ["version", "id", "name", "mode", "style", "colors"])
  )
    return false;
  return (
    v.version === 1 &&
    typeof v.id === "string" &&
    v.id.length <= 80 &&
    typeof v.name === "string" &&
    v.name.trim().length > 0 &&
    [...v.name].length <= 60 &&
    ![...v.name].some(
      (c) =>
        c.charCodeAt(0) < 32 ||
        (c.charCodeAt(0) >= 127 && c.charCodeAt(0) <= 159),
    ) &&
    (v.mode === "dark" || v.mode === "light") &&
    typeof v.style === "string" &&
    ["clear", "soft", "terminal"].includes(v.style) &&
    object(v.colors) &&
    keysAre(v.colors, COLOR_KEYS) &&
    COLOR_KEYS.every((k) => {
      const value = (v.colors as Record<string, unknown>)[k];
      return typeof value === "string" && /^#[\da-f]{6}$/i.test(value);
    })
  );
}
export function isCustomId(id: string): boolean {
  return /^custom-[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i.test(
    id,
  );
}
export function isThemeDocument(
  v: unknown,
  builtinIds: readonly string[],
): v is ThemeDocument {
  if (
    !object(v) ||
    !keysAre(v, ["version", "revision", "active", "customThemes"]) ||
    v.version !== 1 ||
    !Number.isSafeInteger(v.revision) ||
    Number(v.revision) < 0 ||
    typeof v.active !== "string" ||
    !Array.isArray(v.customThemes) ||
    v.customThemes.length > MAX_THEMES
  )
    return false;
  if (!v.customThemes.every((t) => isTheme(t) && isCustomId(t.id)))
    return false;
  const ids = v.customThemes.map((t) => (t as ThemeDefinition).id);
  return (
    new Set(ids).size === ids.length &&
    (builtinIds.includes(v.active) || ids.includes(v.active))
  );
}
export function importTheme(text: string): ThemeDefinition {
  if (new TextEncoder().encode(text).length > MAX_IMPORT_BYTES)
    throw new Error("Die Datei darf höchstens 64 KiB groß sein.");
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("Die Datei enthält kein gültiges JSON.");
  }
  if (!isTheme(parsed))
    throw new Error(
      "Das Theme-Format ist ungültig oder wird noch nicht unterstützt.",
    );
  return { ...parsed, id: `custom-${randomUuid()}` };
}
export function luminance(hex: string): number {
  const rgb = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((n) => (n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4));
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
}
export function contrast(a: string, b: string): number {
  const x = luminance(a),
    y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
export function readableInk(background: string): string {
  return contrast("#ffffff", background) > contrast("#111111", background)
    ? "#ffffff"
    : "#111111";
}
export function mix(a: string, b: string, amount: number): string {
  return (
    "#" +
    [1, 3, 5]
      .map((i) =>
        Math.round(
          parseInt(a.slice(i, i + 2), 16) * (1 - amount) +
            parseInt(b.slice(i, i + 2), 16) * amount,
        )
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
  );
}
export function ensureContrast(
  foreground: string,
  backgrounds: string[],
  ratio = 4.5,
): string {
  if (backgrounds.every((bg) => contrast(foreground, bg) >= ratio))
    return foreground;
  const target = ["#ffffff", "#111111"].sort(
    (a, b) =>
      Math.min(...backgrounds.map((bg) => contrast(b, bg))) -
      Math.min(...backgrounds.map((bg) => contrast(a, bg))),
  )[0];
  for (let step = 1; step <= 100; step++) {
    const next = mix(foreground, target, step / 100);
    if (backgrounds.every((bg) => contrast(next, bg) >= ratio)) return next;
  }
  return target;
}
/** A colour on the HSL wheel, as `#rrggbb`. */
export function hsl(
  hue: number,
  saturation: number,
  lightness: number,
): string {
  const k = (n: number) => (n + hue / 30) % 12;
  const a = saturation * Math.min(lightness, 1 - lightness);
  return (
    "#" +
    [0, 8, 4]
      .map((n) =>
        Math.round(
          (lightness - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))) * 255,
        )
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
  );
}
// Server tiles without artwork: muted hues that sit quietly next to the
// accent. Dark themes use deep tints, light themes pale ones.
const TILE_HUES = [18, 42, 88, 145, 188, 214, 258, 322];
/**
 * Every colour the interface derives from a theme's eight editable ones.
 * The accent stays a fill (buttons, switches, markers); text and icons in
 * the accent colour use `accentText`, which is darkened or lightened until
 * it reads on every surface.
 */
export function derivedColors(theme: ThemeDefinition) {
  const c = theme.colors,
    dark = theme.mode === "dark";
  const elevated = dark
    ? mix(c.panel, c.text, 0.04)
    : mix(c.background, "#ffffff", 0.6);
  const selected = mix(c.panel, c.accent, 0.18);
  const hover = mix(c.panel, c.text, 0.08);
  const online = ensureContrast(
    dark ? "#85d69b" : "#287a45",
    [c.background, c.panel, c.surface],
    3,
  );
  const accentText = ensureContrast(c.accent, [
    c.background,
    c.panel,
    c.surface,
    selected,
  ]);
  const muted = ensureContrast(c.muted, [c.panel, c.surface, hover]);
  return {
    elevated,
    selected,
    hover,
    muted,
    subtle: ensureContrast(mix(muted, c.background, 0.25), [c.panel, elevated]),
    borderStrong: ensureContrast(
      mix(c.border, c.text, 0.35),
      [c.background, c.panel, c.surface, elevated],
      3,
    ),
    accentInk: readableInk(c.accent),
    accentText,
    online,
    onlineText: ensureContrast(online, [c.panel, c.surface]),
    warning: ensureContrast("#d8a657", [c.background, c.panel, c.surface], 3),
    danger: ensureContrast(dark ? "#f39b98" : "#a12b29", [
      c.background,
      c.panel,
      c.surface,
    ]),
    dangerBg: dark ? "#8f302d" : "#b33b38",
    tiles: TILE_HUES.map((hue) => {
      const tile = dark ? hsl(hue, 0.3, 0.3) : hsl(hue, 0.45, 0.85);
      for (let step = 0; step <= 100; step += 5) {
        const next = mix(tile, c.background, step / 100);
        if (contrast(c.text, next) >= 4.5) return next;
      }
      return c.background;
    }),
  };
}
export function contrastIssues(theme: ThemeDefinition): string[] {
  const c = theme.colors,
    d = derivedColors(theme),
    backgrounds = [c.background, c.panel, c.surface, c.rail];
  const issues = [];
  if (backgrounds.some((bg) => contrast(c.text, bg) < 4.5))
    issues.push("Text ist auf mindestens einer Fläche schwer lesbar.");
  if (backgrounds.some((bg) => contrast(c.muted, bg) < 4.5))
    issues.push("Sekundärtext braucht mehr Kontrast.");
  if (contrast(d.accentInk, c.accent) < 4.5)
    issues.push("Schrift auf der Akzentfarbe ist schwer lesbar.");
  if (
    [c.background, c.panel, c.surface, d.selected].some(
      (bg) => contrast(d.accentText, bg) < 4.5,
    )
  )
    issues.push("Links in der Akzentfarbe heben sich zu wenig ab.");
  if (
    [c.background, c.panel, c.surface, d.elevated].some(
      (bg) => contrast(d.borderStrong, bg) < 3,
    )
  )
    issues.push("Rahmen von Eingabefeldern heben sich zu wenig ab.");
  return issues;
}
/**
 * Raises text and secondary text to 4.5:1. The accent is left alone unless
 * neither dark nor light ink is readable on it: it is a fill, and accent
 * text gets its own derived colour.
 */
export function fixContrast(theme: ThemeDefinition): ThemeDefinition {
  const colors = { ...theme.colors };
  const backgrounds = [
    colors.background,
    colors.panel,
    colors.surface,
    colors.rail,
  ];
  colors.text = ensureContrast(colors.text, backgrounds);
  colors.muted = ensureContrast(colors.muted, backgrounds);
  const ink = readableInk(colors.accent);
  if (contrast(ink, colors.accent) < 4.5) {
    const away = ink === "#ffffff" ? "#000000" : "#ffffff";
    const accent = colors.accent;
    for (let step = 1; step <= 100; step++) {
      colors.accent = mix(accent, away, step / 100);
      if (contrast(readableInk(colors.accent), colors.accent) >= 4.5) break;
    }
  }
  return { ...theme, colors };
}
const RADII: Record<ThemeStyle, number[]> = {
  clear: [3, 5, 7, 10, 14],
  soft: [6, 8, 12, 16, 20],
  terminal: [1, 2, 2, 2, 3],
};
export function themeVariables(theme: ThemeDefinition): Record<string, string> {
  const c = theme.colors,
    d = derivedColors(theme),
    dark = theme.mode === "dark";
  const neutral =
    theme.mode === "dark"
      ? [
          c.text,
          c.text,
          c.text,
          c.text,
          c.muted,
          c.muted,
          c.muted,
          c.border,
          c.surface,
          c.panel,
          c.background,
        ]
      : [
          c.background,
          c.panel,
          c.surface,
          c.border,
          c.muted,
          c.muted,
          c.muted,
          c.text,
          c.text,
          c.text,
          c.text,
        ];
  const [xs, sm, md, lg, xl] = RADII[theme.style].map((r) => `${r}px`);
  return {
    ...Object.fromEntries(
      [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950].map((shade, i) => [
        `--color-neutral-${shade}`,
        neutral[i],
      ]),
    ),
    ...Object.fromEntries(
      d.tiles.map((tile, i) => [`--lr-tile-${i + 1}`, tile]),
    ),
    "--font-sans": "var(--lr-font)",
    // Tailwind's rounded-* utilities follow the theme style too.
    "--radius-xs": "var(--lr-radius-xs)",
    "--radius-sm": "var(--lr-radius-sm)",
    "--radius-md": "var(--lr-radius-sm)",
    "--radius-lg": "var(--lr-radius-sm)",
    "--radius-xl": "var(--lr-radius-md)",
    "--radius-2xl": "var(--lr-radius-xl)",
    "--radius-3xl": "var(--lr-radius-xl)",
    "--lr-bg": c.background,
    "--lr-panel": c.panel,
    "--lr-surface": c.surface,
    "--lr-rail": c.rail,
    "--lr-elevated": d.elevated,
    "--lr-text": c.text,
    "--lr-muted": d.muted,
    "--lr-subtle": d.subtle,
    "--lr-border": c.border,
    "--lr-border-strong": d.borderStrong,
    "--lr-accent": c.accent,
    "--lr-accent-ink": d.accentInk,
    "--lr-accent-text": d.accentText,
    "--lr-focus": d.accentText,
    "--lr-hover": d.hover,
    "--lr-selected": d.selected,
    "--lr-online": d.online,
    "--lr-online-text": d.onlineText,
    "--lr-warning": d.warning,
    "--lr-danger": d.danger,
    "--lr-danger-bg": d.dangerBg,
    "--lr-on-danger": "#ffffff",
    "--lr-backdrop": dark ? "rgb(9 12 13 / 0.72)" : "rgb(37 40 41 / 0.45)",
    "--lr-font":
      theme.style === "terminal"
        ? 'ui-monospace, "Cascadia Code", monospace'
        : 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    "--lr-radius-xs": xs,
    "--lr-radius-sm": sm,
    "--lr-radius-md": md,
    "--lr-radius-lg": lg,
    "--lr-radius-xl": xl,
    // Older names, still used across the stylesheets.
    "--lr-radius": md,
    "--lr-radius-small": sm,
    "--lr-shadow-1": dark
      ? "0 8px 24px rgb(0 0 0 / 0.32)"
      : "0 8px 24px rgb(60 45 20 / 0.12)",
    "--lr-shadow-2": dark
      ? "0 18px 48px rgb(0 0 0 / 0.5)"
      : "0 18px 48px rgb(60 45 20 / 0.18)",
    "--lr-shadow-card":
      theme.style === "soft" ? "0 6px 22px #00000012" : "none",
  };
}
