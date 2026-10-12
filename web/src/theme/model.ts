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
export function contrastIssues(theme: ThemeDefinition): string[] {
  const c = theme.colors,
    backgrounds = [c.background, c.panel, c.surface, c.rail];
  const issues = [];
  if (backgrounds.some((bg) => contrast(c.text, bg) < 4.5))
    issues.push("Text ist auf mindestens einer Fläche schwer lesbar.");
  if (backgrounds.some((bg) => contrast(c.muted, bg) < 4.5))
    issues.push("Sekundärtext braucht mehr Kontrast.");
  if (backgrounds.some((bg) => contrast(c.accent, bg) < 3))
    issues.push("Die Akzentfarbe hebt sich zu wenig ab.");
  return issues;
}
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
  colors.accent = ensureContrast(colors.accent, backgrounds, 3);
  return { ...theme, colors };
}
export function themeVariables(theme: ThemeDefinition): Record<string, string> {
  const c = theme.colors;
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
  return {
    ...Object.fromEntries(
      [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950].map((shade, i) => [
        `--color-neutral-${shade}`,
        neutral[i],
      ]),
    ),
    "--font-sans": "var(--lr-font)",
    "--radius-sm": "var(--lr-radius-small)",
    "--radius-md": "var(--lr-radius-small)",
    "--radius-lg": "var(--lr-radius)",
    "--radius-xl": "var(--lr-radius)",
    "--lr-bg": c.background,
    "--lr-panel": c.panel,
    "--lr-surface": c.surface,
    "--lr-rail": c.rail,
    "--lr-text": c.text,
    "--lr-muted": c.muted,
    "--lr-border": c.border,
    "--lr-accent": c.accent,
    "--lr-accent-ink": readableInk(c.accent),
    "--lr-hover": mix(c.panel, c.text, 0.08),
    "--lr-selected": mix(c.panel, c.accent, 0.18),
    "--lr-online": ensureContrast(
      theme.mode === "dark" ? "#85d69b" : "#287a45",
      [c.background, c.panel, c.surface],
      3,
    ),
    "--lr-away": ensureContrast(
      "#d8a657",
      [c.background, c.panel, c.surface],
      3,
    ),
    "--lr-danger": ensureContrast(
      theme.mode === "dark" ? "#f39b98" : "#a12b29",
      [c.background, c.panel, c.surface],
      4.5,
    ),
    "--lr-danger-bg": theme.mode === "dark" ? "#8f302d" : "#b33b38",
    "--lr-font":
      theme.style === "terminal"
        ? 'ui-monospace, "Cascadia Code", monospace'
        : 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    "--lr-radius":
      theme.style === "soft"
        ? "14px"
        : theme.style === "terminal"
          ? "2px"
          : "7px",
    "--lr-radius-small":
      theme.style === "soft"
        ? "8px"
        : theme.style === "terminal"
          ? "1px"
          : "4px",
    "--lr-shadow": theme.style === "soft" ? "0 6px 22px #00000012" : "none",
  };
}
