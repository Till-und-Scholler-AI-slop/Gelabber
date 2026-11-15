// Every field state in four themes, for scripts/field-gallery.mjs (#561).
// One frame per field, the same tokens everywhere; the desktop app's
// counterpart is desktop/ui/src/widgets/gallery.rs.
/* eslint-disable react-refresh/only-export-components -- an entry page, not a module */
import type { CSSProperties } from "react";
import { createRoot } from "react-dom/client";

import "../../src/index.css";
import { SearchIcon } from "../../src/components/Icons.tsx";
import { FieldShell } from "../../src/components/ui/FieldShell.tsx";
import { Input } from "../../src/components/ui/Input.tsx";
import { Select } from "../../src/components/ui/Select.tsx";
import { Textarea } from "../../src/components/ui/Textarea.tsx";
import { themeVariables } from "../../src/theme/model.ts";
import { BUILTIN_THEMES } from "../../src/theme/presets.ts";

/** Dark and light (clear), and one each of the soft and terminal styles. */
const THEMES = [
  BUILTIN_THEMES.find((t) => t.id === "dark")!,
  BUILTIN_THEMES.find((t) => t.id === "light")!,
  BUILTIN_THEMES.find((t) => t.style === "soft")!,
  BUILTIN_THEMES.find((t) => t.style === "terminal")!,
];

const cell: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "minmax(0, 1fr)",
  gap: 6,
  width: 260,
  padding: 8,
};

function Fields({ id }: { id: string }) {
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "flex-start",
        gap: 4,
      }}
    >
      <div style={cell} data-field={`${id}-input`}>
        <Input placeholder="Status" aria-label="Status" />
      </div>
      <div style={cell} data-field={`${id}-filled`}>
        <Input defaultValue="Bin im Call" aria-label="Gefüllt" />
      </div>
      <div style={cell} data-field={`${id}-invalid`}>
        <Input defaultValue="zu lang" invalid aria-label="Fehler" />
      </div>
      <div style={cell} data-field={`${id}-disabled`}>
        <Input defaultValue="gesperrt" disabled aria-label="Gesperrt" />
      </div>
      <div style={cell} data-field={`${id}-select`}>
        <Select aria-label="Auswahl" defaultValue="b">
          <option value="a">Erste</option>
          <option value="b">Zweite</option>
        </Select>
      </div>
      <div style={cell} data-field={`${id}-search`}>
        <FieldShell
          as="label"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            height: 30,
            padding: "0 10px",
            boxSizing: "border-box",
            width: "100%",
          }}
        >
          <SearchIcon size={16} />
          <Input
            bare
            type="search"
            defaultValue="hallo"
            aria-label="Suchen"
            style={{ flex: 1, height: "100%" }}
          />
        </FieldShell>
      </div>
      <div style={cell} data-field={`${id}-textarea`}>
        <Textarea defaultValue="Eine Nachricht" aria-label="Text" rows={2} />
      </div>
      <div style={cell} data-field={`${id}-color`}>
        <input
          type="color"
          className="ui-color"
          defaultValue="#f3b752"
          aria-label="Farbe"
          style={{ width: 36, height: 36 }}
        />
      </div>
    </div>
  );
}

function Gallery() {
  return (
    <>
      {THEMES.map((theme) => (
        <section
          key={theme.id}
          data-theme-id={theme.id}
          style={
            {
              ...themeVariables(theme),
              background: "var(--lr-bg)",
              color: "var(--lr-text)",
              fontFamily: "var(--lr-font)",
              padding: 8,
            } as CSSProperties
          }
        >
          <Fields id={theme.id} />
        </section>
      ))}
    </>
  );
}

createRoot(document.getElementById("root")!).render(<Gallery />);
