import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Picker, type PickerOption } from "./Picker.tsx";
import { ShortcutSheet } from "./QuickSwitcher.tsx";

// The switcher's markup: a combobox over a listbox. Node has no <dialog>
// and no keyboard; the ranking is in quickSwitch.test.ts.
const option = (id: string, detail: string | null = null) =>
  ({
    id,
    value: id,
    label: `#${id}`,
    detail,
    icon: <b />,
  }) satisfies PickerOption<string>;

const picker = (options: PickerOption<string>[], query = "") =>
  renderToStaticMarkup(
    <Picker
      label="Server, Kanal oder Unterhaltung"
      placeholder="Server, Kanal oder Unterhaltung suchen"
      query={query}
      onQuery={() => {}}
      options={options}
      onPick={() => {}}
      empty={`Nichts gefunden für „${query}“.`}
    />,
  );

describe("quick switcher", () => {
  it("is a combobox that points at its highlighted option", () => {
    const html = picker([option("termine", "Spieleabend"), option("memes")]);
    const listId = /<ul id="([^"]+)" role="listbox"/.exec(html)?.[1];
    expect(listId).toBeTruthy();
    expect(html).toContain('role="combobox"');
    expect(html).toContain(`aria-controls="${listId}"`);
    expect(html).toContain('aria-expanded="true"');
    const active = /aria-activedescendant="([^"]+)"/.exec(html)?.[1];
    expect(html).toMatch(
      new RegExp(`id="${active}" role="option" aria-selected="true"`),
    );
    expect(html.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(html).toContain("Spieleabend");
  });

  it("says how many match", () => {
    expect(picker([option("termine"), option("memes")])).toMatch(
      /role="status"[^>]*>2 Treffer</,
    );
  });

  it("says so when nothing matches, also to screen readers", () => {
    const html = picker([], "zzz");
    expect(html).not.toContain('role="listbox"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain("aria-activedescendant");
    expect(html).toMatch(
      /class="shell-picker-empty" role="status">Nichts gefunden für „zzz“.</,
    );
  });

  it("lists every shortcut with its keys", () => {
    const html = renderToStaticMarkup(
      <ShortcutSheet open onClose={() => {}} />,
    );
    expect(html).toContain("Tastenkürzel");
    expect(html).toContain(
      "<dt><kbd>Strg</kbd><kbd>K</kbd></dt><dd>Server, Kanal oder Unterhaltung öffnen</dd>",
    );
    expect(html).toContain(
      "<dt><kbd>Alt</kbd><kbd>Umschalt</kbd><kbd>↑ / ↓</kbd></dt>",
    );
  });
});
