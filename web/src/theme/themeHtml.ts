import defaults from "./defaults.json";

/**
 * index.html paints the default theme before the app loads. Its colours are
 * written as `%theme.dark.background%` and filled in at build time
 * (vite.config.ts) from the file the built-in presets read, so there is one
 * copy of them.
 */
export function fillThemeColours(html: string): string {
  const colours: Record<string, Record<string, string> | undefined> = defaults;
  return html.replace(/%theme\.(\w+)\.(\w+)%/g, (match, mode, key) => {
    const value = colours[mode]?.[key];
    if (!value) throw new Error(`index.html: unknown colour ${match}`);
    return value;
  });
}
