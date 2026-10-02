import { useEffect, useRef, useState } from "react";
import { ScreenIcon } from "../components/Icons.tsx";
import { useBlocker } from "@tanstack/react-router";
import { BUILTIN_THEMES, builtinTheme } from "./presets.ts";
import {
  COLOR_KEYS,
  contrastIssues,
  fixContrast,
  importTheme,
  MAX_IMPORT_BYTES,
  MAX_THEMES,
  type ThemeDefinition,
} from "./model.ts";
import {
  changeTheme,
  discardPendingTheme,
  retryThemes,
  syncThemes,
  useAccountThemes,
} from "./account.ts";
import { ThemePreview } from "./ThemePreview.tsx";
import "./themes.css";

const labels = {
  background: "Hintergrund",
  panel: "Seitenleiste",
  surface: "Flächen",
  rail: "Community-Leiste",
  text: "Text",
  muted: "Sekundärtext",
  accent: "Akzent",
  border: "Rahmen",
};
const styles = { clear: "Klar", soft: "Weich", terminal: "Terminal" } as const;
function copyTheme(theme: ThemeDefinition): ThemeDefinition {
  return {
    ...theme,
    id: `custom-${crypto.randomUUID()}`,
    name: `${theme.name.slice(0, 48)} – Eigene`,
    colors: { ...theme.colors },
  };
}
function exportTheme(theme: ThemeDefinition) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(theme, null, 2)], { type: "application/json" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = `${theme.name.replace(/[^\p{L}\p{N}-]+/gu, "-")}.gelabber-theme.json`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function ColorField({
  name,
  value,
  onChange,
}: {
  name: string;
  value: string;
  onChange: (color: string) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  return (
    <label className="theme-color-field">
      <span>{name}</span>
      <div>
        <input
          type="color"
          aria-label={`${name} wählen`}
          value={value}
          onChange={(e) => {
            setEditing(null);
            onChange(e.target.value);
          }}
        />
        <input
          aria-label={`${name} Hexwert`}
          value={editing ?? value}
          pattern="#[0-9a-fA-F]{6}"
          required
          maxLength={7}
          onChange={(e) => {
            setEditing(e.target.value);
            if (/^#[0-9a-fA-F]{6}$/.test(e.target.value))
              onChange(e.target.value);
          }}
          onFocus={(e) => e.target.select()}
          onBlur={() => setEditing(null)}
        />
      </div>
    </label>
  );
}

export function ThemesPanel({
  onDirtyChange,
}: {
  onDirtyChange: (dirty: boolean) => void;
}) {
  const state = useAccountThemes();
  const [filter, setFilter] = useState("all");
  const [selected, setSelected] = useState(state.doc.active);
  const [draft, setDraft] = useState<ThemeDefinition | null>(null);
  const [baseline, setBaseline] = useState("");
  const [notice, setNotice] = useState("");
  const editor = useRef<HTMLDivElement>(null);
  const file = useRef<HTMLInputElement>(null);
  const gallery = useRef<HTMLDivElement>(null);
  const all = [...BUILTIN_THEMES, ...state.doc.customThemes];
  const chosen =
    selected === "system"
      ? builtinTheme(
          matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
        )
      : (all.find((t) => t.id === selected) ?? builtinTheme("dark"));
  const shown = draft ?? chosen;
  const dirty = draft !== null && JSON.stringify(draft) !== baseline;
  const blocked =
    state.status === "saving" ||
    state.status === "loading" ||
    state.status === "error" ||
    !!state.pending;
  const issues = contrastIssues(shown);
  const canCreate = state.doc.customThemes.length < MAX_THEMES;
  useBlocker({
    shouldBlockFn: () =>
      dirty && !window.confirm("Ungespeicherten Theme-Entwurf verwerfen?"),
    enableBeforeUnload: dirty,
  });
  useEffect(() => {
    onDirtyChange(dirty);
    return () => onDirtyChange(false);
  }, [dirty, onDirtyChange]);
  useEffect(() => {
    void syncThemes();
  }, []);
  const abandon = () =>
    !dirty || window.confirm("Ungespeicherten Theme-Entwurf verwerfen?");
  const open = (theme: ThemeDefinition, isNew: boolean) => {
    if (!abandon()) return;
    setDraft(theme);
    setBaseline(isNew ? "" : JSON.stringify(theme));
    setNotice("");
    requestAnimationFrame(() => {
      editor.current?.scrollIntoView({ block: "start", behavior: "instant" });
      editor.current?.querySelector<HTMLInputElement>("input")?.focus();
    });
  };
  const choose = (id: string) => {
    if (!abandon()) return;
    setDraft(null);
    setSelected(id);
    setNotice("");
    requestAnimationFrame(() => {
      editor.current?.scrollIntoView({ block: "start", behavior: "instant" });
      editor.current?.focus();
    });
  };
  return (
    <div className="themes-panel">
      <div className="themes-toolbar">
        <div>
          <strong>Dein Look. Dein Gelabber.</strong>
          <p>Wähle eine Farbwelt oder gestalte deine eigene.</p>
        </div>
        <div className="themes-actions">
          <button
            disabled={!canCreate || blocked}
            onClick={() => open(copyTheme(chosen), true)}
          >
            + Eigenes Theme
          </button>
          <button
            disabled={!canCreate || blocked}
            onClick={() => file.current?.click()}
          >
            Importieren
          </button>
        </div>
      </div>
      <input
        ref={file}
        className="theme-file"
        type="file"
        accept=".json,.gelabber-theme.json,application/json"
        aria-label="Theme-Datei importieren"
        onChange={async (e) => {
          const selectedFile = e.target.files?.[0];
          e.target.value = "";
          if (!selectedFile) return;
          if (selectedFile.size > MAX_IMPORT_BYTES) {
            setNotice("Die Datei darf höchstens 64 KiB groß sein.");
            return;
          }
          try {
            const theme = importTheme(await selectedFile.text());
            open(theme, true);
          } catch (error) {
            setNotice(
              error instanceof Error ? error.message : "Import fehlgeschlagen.",
            );
          }
        }}
      />
      {(state.error || state.status === "saving") && (
        <div className="themes-sync" role="status">
          <span>
            {state.status === "saving"
              ? "Wird mit deinem Account synchronisiert …"
              : state.error}
          </span>
          {state.status !== "saving" && (
            <div className="themes-actions">
              <button onClick={() => void retryThemes()}>
                {state.pending ? "Erneut anwenden" : "Erneut versuchen"}
              </button>
              {state.pending && (
                <button
                  onClick={() => {
                    if (
                      window.confirm(
                        "Lokale Änderung verwerfen und Account-Stand laden?",
                      )
                    )
                      void discardPendingTheme();
                  }}
                >
                  Account-Stand laden
                </button>
              )}
            </div>
          )}
        </div>
      )}
      {notice && (
        <p className="themes-sync" role="status">
          {notice}
        </p>
      )}
      <div className="themes-filter" role="group" aria-label="Themes filtern">
        {[
          ["all", "Alle"],
          ["light", "Hell"],
          ["dark", "Dunkel"],
          ["own", "Eigene"],
        ].map(([id, label]) => (
          <button
            key={id}
            aria-pressed={filter === id}
            onClick={() => setFilter(id)}
          >
            {label}
          </button>
        ))}
      </div>
      <div ref={gallery} className="themes-gallery" aria-label="Theme-Galerie">
        {filter === "all" && (
          <button
            className="theme-card theme-system"
            aria-pressed={selected === "system"}
            onClick={() => choose("system")}
          >
            <span className="theme-system-symbol">
              <ScreenIcon size={32} />
            </span>
            <strong>System</strong>
            <span>Gelabber Hell oder Dunkel · wie dein Gerät</span>
            {state.doc.active === "system" && (
              <b className="theme-active">Aktiv</b>
            )}
          </button>
        )}
        {all
          .filter(
            (t) =>
              filter === "all" ||
              (filter === "own"
                ? t.id.startsWith("custom-")
                : t.mode === filter),
          )
          .map((theme) => (
            <button
              key={theme.id}
              className="theme-card"
              aria-pressed={selected === theme.id}
              onClick={() => choose(theme.id)}
            >
              <ThemePreview theme={theme} small />
              <div className="theme-card-description">
                <strong>{theme.name}</strong>
                <span>
                  {theme.mode === "dark" ? "Dunkel" : "Hell"} ·{" "}
                  {styles[theme.style]}
                </span>
                <div className="theme-swatches">
                  {[
                    theme.colors.background,
                    theme.colors.surface,
                    theme.colors.text,
                    theme.colors.accent,
                  ].map((color, i) => (
                    <i key={i} style={{ background: color }} />
                  ))}
                </div>
                {state.doc.active === theme.id && (
                  <b className="theme-active">Aktiv</b>
                )}
              </div>
            </button>
          ))}
      </div>
      {filter === "own" && state.doc.customThemes.length === 0 && (
        <p className="themes-empty">
          Hier ist Platz für deinen Stil. Starte mit einer Vorlage oder
          importiere ein Theme.
        </p>
      )}
      <div ref={editor} tabIndex={-1} className="theme-workbench">
        <header>
          <button
            type="button"
            onClick={() => {
              gallery.current?.scrollIntoView({
                block: "start",
                behavior: "instant",
              });
              gallery.current
                ?.querySelector<HTMLButtonElement>('[aria-pressed="true"]')
                ?.focus();
            }}
          >
            Zur Galerie
          </button>
          <div>
            <span className="theme-eyebrow">
              {draft ? "DEIN ENTWURF" : "VORSCHAU"}
            </span>
            <h3>
              {draft
                ? draft.name || "Neues Theme"
                : selected === "system"
                  ? "Wie dein Gerät"
                  : chosen.name}
            </h3>
          </div>
          {!draft && (
            <span className="theme-preview-hint">
              Dein aktives Theme bleibt bis zum Anwenden erhalten.
            </span>
          )}
        </header>
        <div className="theme-workbench-grid">
          <div className="theme-large-preview">
            <ThemePreview
              theme={
                selected === "system" && !draft
                  ? builtinTheme(
                      matchMedia("(prefers-color-scheme: dark)").matches
                        ? "dark"
                        : "light",
                    )
                  : shown
              }
            />
            {issues.length > 0 && (
              <div className="theme-contrast" role="status">
                <strong>Kontrast prüfen</strong>
                {issues.map((issue) => (
                  <p key={issue}>{issue}</p>
                ))}
                {draft && (
                  <button onClick={() => setDraft(fixContrast(draft))}>
                    Kontraste verbessern
                  </button>
                )}
              </div>
            )}
          </div>
          {draft && (
            <form
              className="theme-editor"
              onSubmit={async (e) => {
                e.preventDefault();
                if (
                  await changeTheme({
                    kind: "save",
                    theme: { ...draft, name: draft.name.trim() },
                  })
                ) {
                  setSelected(draft.id);
                  setDraft(null);
                  setNotice("Theme gespeichert und angewendet.");
                }
              }}
            >
              <label>
                Name
                <input
                  required
                  maxLength={60}
                  value={draft.name}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              </label>
              <div className="theme-editor-pair">
                <label>
                  Modus
                  <select
                    value={draft.mode}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        mode: e.target.value as ThemeDefinition["mode"],
                      })
                    }
                  >
                    <option value="dark">Dunkel</option>
                    <option value="light">Hell</option>
                  </select>
                </label>
                <label>
                  Stil
                  <select
                    value={draft.style}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        style: e.target.value as ThemeDefinition["style"],
                      })
                    }
                  >
                    {Object.entries(styles).map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <fieldset>
                <legend>Farben</legend>
                <div className="theme-color-grid">
                  {COLOR_KEYS.map((key) => (
                    <ColorField
                      key={key}
                      name={labels[key]}
                      value={draft.colors[key]}
                      onChange={(color) =>
                        setDraft({
                          ...draft,
                          colors: { ...draft.colors, [key]: color },
                        })
                      }
                    />
                  ))}
                </div>
              </fieldset>
              <div className="themes-actions">
                <button
                  className="theme-primary"
                  type="submit"
                  disabled={blocked || !draft.name.trim()}
                >
                  Speichern und anwenden
                </button>
                <button
                  type="button"
                  onClick={() => {
                    if (abandon()) setDraft(null);
                  }}
                >
                  Abbrechen
                </button>
              </div>
            </form>
          )}
        </div>
        {!draft && (
          <div className="themes-actions theme-preview-actions">
            <button
              className="theme-primary"
              disabled={blocked || selected === state.doc.active}
              onClick={async () => {
                if (await changeTheme({ kind: "apply", id: selected }))
                  setNotice("Theme angewendet.");
              }}
            >
              {selected === state.doc.active ? "Aktives Theme" : "Anwenden"}
            </button>
            <button
              disabled={!canCreate || blocked}
              onClick={() => open(copyTheme(chosen), true)}
            >
              Als Vorlage verwenden
            </button>
            {chosen.id.startsWith("custom-") && selected !== "system" && (
              <>
                <button
                  disabled={blocked}
                  onClick={() =>
                    open({ ...chosen, colors: { ...chosen.colors } }, false)
                  }
                >
                  Bearbeiten
                </button>
                <button
                  disabled={blocked}
                  onClick={async () => {
                    if (
                      window.confirm(`„${chosen.name}“ löschen?`) &&
                      (await changeTheme({ kind: "delete", id: chosen.id }))
                    ) {
                      setSelected("dark");
                      setNotice("Theme gelöscht.");
                    }
                  }}
                >
                  Löschen
                </button>
              </>
            )}
            <button onClick={() => exportTheme(chosen)}>Exportieren</button>
          </div>
        )}
      </div>
      <p className="themes-footnote">
        {state.status === "ready"
          ? "Mit deinem Account synchronisiert."
          : "Deine Themes gehören zu deinem Account."}{" "}
        Paletten nach{" "}
        <a
          href="https://github.com/omacom/omarchy/tree/quattro/themes"
          target="_blank"
          rel="noreferrer"
        >
          Omarchy
        </a>
        , für Gelabber angepasst.
      </p>
    </div>
  );
}
