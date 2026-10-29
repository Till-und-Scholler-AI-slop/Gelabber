import { useEffect, useRef, useState } from "react";
import { CheckIcon, ScreenIcon } from "../components/Icons.tsx";
import { useBlocker } from "@tanstack/react-router";
import { confirmAction } from "../components/confirm.ts";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import type { InlineStatusValue } from "../components/ui/useInlineStatus.ts";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { nextChipIndex } from "../components/ui/chipKeys.ts";
import { randomUuid } from "../lib/uuid.ts";
import { blockThemeLeave, confirmDiscardThemeDraft } from "./discardDraft.ts";
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
import {
  endThemePreview,
  escapeEndsPreview,
  galleryPreview,
  previewTheme,
} from "./preview.ts";
import { ThemePreview } from "./ThemePreview.tsx";
import "./themes.css";

const labels = {
  background: "Hintergrund",
  panel: "Seitenleiste",
  surface: "Flächen",
  rail: "Serverleiste",
  text: "Text",
  muted: "Sekundärtext",
  accent: "Akzent",
  border: "Rahmen",
};
const styles = { clear: "Klar", soft: "Weich", terminal: "Terminal" } as const;
const FILTERS = [
  { value: "all", label: "Alle" },
  { value: "light", label: "Hell" },
  { value: "dark", label: "Dunkel" },
  { value: "own", label: "Eigene" },
] as const;
type Filter = (typeof FILTERS)[number]["value"];
const systemDark = () => matchMedia("(prefers-color-scheme: dark)").matches;
function copyTheme(theme: ThemeDefinition): ThemeDefinition {
  return {
    ...theme,
    id: `custom-${randomUuid()}`,
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

export function ThemesPanel() {
  const state = useAccountThemes();
  const [filter, setFilter] = useState<Filter>("all");
  // The card tried on, or null for the account's theme, which then follows
  // a sync or another device without turning into a preview of the old one.
  const [picked, setPicked] = useState<string | null>(null);
  const [draft, setDraft] = useState<ThemeDefinition | null>(null);
  const [baseline, setBaseline] = useState("");
  const [notice, setNotice] = useState<InlineStatusValue>(null);
  const file = useRef<HTMLInputElement>(null);
  const cards = useRef<(HTMLButtonElement | null)[]>([]);
  const all = [...BUILTIN_THEMES, ...state.doc.customThemes];
  const active = state.doc.active;
  const selected = picked ?? active;
  const themeName = (id: string) =>
    id === "system"
      ? "System"
      : (all.find((t) => t.id === id)?.name ?? builtinTheme("dark").name);
  const chosen =
    selected === "system"
      ? builtinTheme(systemDark() ? "dark" : "light")
      : (all.find((t) => t.id === selected) ?? builtinTheme("dark"));
  const shown = draft ?? chosen;
  const dirty = draft !== null && JSON.stringify(draft) !== baseline;
  const preview = galleryPreview(picked, active, draft !== null);
  const previewing = preview !== null;
  const blocked =
    state.status === "saving" ||
    state.status === "loading" ||
    state.status === "error" ||
    !!state.pending;
  const issues = contrastIssues(shown);
  const canCreate = state.doc.customThemes.length < MAX_THEMES;
  const options: { id: string; theme: ThemeDefinition | null }[] = [
    ...(filter === "all" ? [{ id: "system", theme: null }] : []),
    ...all
      .filter(
        (t) =>
          filter === "all" ||
          (filter === "own" ? t.id.startsWith("custom-") : t.mode === filter),
      )
      .map((theme) => ({ id: theme.id, theme })),
  ];
  const checkedIndex = options.findIndex((option) => option.id === selected);
  useBlocker({
    shouldBlockFn: () => blockThemeLeave(dirty),
    enableBeforeUnload: dirty,
  });
  useEffect(() => {
    void syncThemes();
  }, []);
  // The gallery tries a theme on across the whole app. Leaving settings or
  // this area puts the account's theme back.
  useEffect(() => {
    if (preview) previewTheme(preview);
    else endThemePreview();
  }, [preview]);
  useEffect(() => endThemePreview, []);
  const revert = () => {
    setPicked(null);
    setNotice(null);
  };
  useEffect(() => {
    if (!previewing) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!escapeEndsPreview(event)) return;
      event.preventDefault();
      setPicked(null);
      setNotice(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [previewing]);
  const abandon = async () => !dirty || (await confirmDiscardThemeDraft());
  const open = async (theme: ThemeDefinition, isNew: boolean) => {
    if (!(await abandon())) return;
    setDraft(theme);
    setBaseline(isNew ? "" : JSON.stringify(theme));
    setNotice(null);
  };
  // Choosing only previews: no scrolling, and focus stays on the card.
  const choose = async (id: string) => {
    if (!(await abandon())) return;
    setDraft(null);
    setPicked(id === active ? null : id);
    setNotice(null);
  };
  const keep = async () => {
    const name = themeName(selected);
    if (await changeTheme({ kind: "apply", id: selected })) {
      setPicked(null);
      setNotice({
        tone: "success",
        message: `„${name}“ angewendet.`,
      });
    } else
      setNotice({
        tone: "error",
        message: "Nicht gespeichert – erneut versuchen",
      });
  };
  return (
    <div className="themes-panel">
      <div className="themes-toolbar">
        <div className="themes-actions">
          <Button
            size="sm"
            disabled={!canCreate || blocked}
            onClick={() => open(copyTheme(chosen), true)}
          >
            + Eigenes Theme
          </Button>
          <Button
            size="sm"
            disabled={!canCreate || blocked}
            onClick={() => file.current?.click()}
          >
            Importieren
          </Button>
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
            setNotice({
              tone: "error",
              message: "Die Datei darf höchstens 64 KiB groß sein.",
            });
            return;
          }
          try {
            const theme = importTheme(await selectedFile.text());
            void open(theme, true);
          } catch (error) {
            setNotice({
              tone: "error",
              message:
                error instanceof Error
                  ? error.message
                  : "Import fehlgeschlagen.",
            });
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
              <Button size="sm" onClick={() => void retryThemes()}>
                {state.pending ? "Erneut anwenden" : "Erneut versuchen"}
              </Button>
              {state.pending && (
                <Button
                  size="sm"
                  onClick={async () => {
                    if (
                      await confirmAction({
                        title: "Lokale Änderung verwerfen?",
                        message:
                          "Gelabber lädt dann wieder den Theme-Stand deines Accounts.",
                        confirmLabel: "Account-Stand laden",
                      })
                    )
                      void discardPendingTheme();
                  }}
                >
                  Account-Stand laden
                </Button>
              )}
            </div>
          )}
        </div>
      )}
      <InlineStatus className="themes-notice" status={notice} />
      <ChipGroup
        className="themes-filter"
        label="Themes filtern"
        options={FILTERS}
        value={filter}
        onChange={setFilter}
      />
      <div className="themes-layout">
        <div>
          <div className="themes-gallery" role="radiogroup" aria-label="Themes">
            {options.map(({ id, theme }, index) => (
              <button
                key={id}
                ref={(element) => {
                  cards.current[index] = element;
                }}
                type="button"
                role="radio"
                aria-checked={selected === id}
                tabIndex={
                  index === (checkedIndex >= 0 ? checkedIndex : 0) ? 0 : -1
                }
                className={theme ? "theme-card" : "theme-card theme-system"}
                onClick={() => void choose(id)}
                onKeyDown={(event) => {
                  const next = nextChipIndex(
                    event.key,
                    index,
                    options.map(() => false),
                  );
                  if (next === null) return;
                  event.preventDefault();
                  cards.current[next]?.focus();
                  void choose(options[next].id);
                }}
              >
                {theme ? (
                  <>
                    <ThemePreview theme={theme} small />
                    <span className="theme-card-description">
                      <strong>{theme.name}</strong>
                      <span>
                        {theme.mode === "dark" ? "Dunkel" : "Hell"} ·{" "}
                        {styles[theme.style]}
                      </span>
                      <span className="theme-swatches" aria-hidden="true">
                        {[
                          theme.colors.background,
                          theme.colors.surface,
                          theme.colors.text,
                          theme.colors.accent,
                        ].map((color, i) => (
                          <i key={i} style={{ background: color }} />
                        ))}
                      </span>
                    </span>
                  </>
                ) : (
                  <>
                    <span className="theme-system-symbol">
                      <ScreenIcon size={32} />
                    </span>
                    <strong>System</strong>
                    <span>Folgt dem Gerät</span>
                  </>
                )}
                {active === id && <b className="theme-active">Aktiv</b>}
                {selected === id && (
                  <span className="theme-check" aria-hidden="true">
                    <CheckIcon size={14} />
                  </span>
                )}
              </button>
            ))}
          </div>
          {filter === "own" && state.doc.customThemes.length === 0 && (
            <p className="themes-empty">Keine eigenen Themes.</p>
          )}
        </div>
        <div className="theme-workbench">
          <header>
            <span className="theme-eyebrow">
              {draft ? "Dein Entwurf" : "Vorschau"}
            </span>
            <h3>
              {draft
                ? draft.name || "Neues Theme"
                : selected === "system"
                  ? "Wie dein Gerät"
                  : chosen.name}
            </h3>
          </header>
          <div className="theme-large-preview">
            <ThemePreview
              theme={
                selected === "system" && !draft
                  ? builtinTheme(systemDark() ? "dark" : "light")
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
                  <Button
                    size="sm"
                    onClick={() => setDraft(fixContrast(draft))}
                  >
                    Kontraste verbessern
                  </Button>
                )}
              </div>
            )}
          </div>
          {draft ? (
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
                  setPicked(null);
                  setDraft(null);
                  setNotice({
                    tone: "success",
                    message: "Theme gespeichert und angewendet.",
                  });
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
                <Button
                  variant="primary"
                  type="submit"
                  disabled={blocked || !draft.name.trim()}
                >
                  Speichern und anwenden
                </Button>
                <Button
                  variant="ghost"
                  onClick={async () => {
                    if (await abandon()) setDraft(null);
                  }}
                >
                  Abbrechen
                </Button>
              </div>
            </form>
          ) : (
            <div className="themes-actions">
              <Button
                size="sm"
                disabled={!canCreate || blocked}
                onClick={() => open(copyTheme(chosen), true)}
              >
                Als Vorlage verwenden
              </Button>
              {chosen.id.startsWith("custom-") && selected !== "system" && (
                <>
                  <Button
                    size="sm"
                    disabled={blocked}
                    onClick={() =>
                      open({ ...chosen, colors: { ...chosen.colors } }, false)
                    }
                  >
                    Bearbeiten
                  </Button>
                  <Button
                    size="sm"
                    variant="danger-outline"
                    disabled={blocked}
                    onClick={async () => {
                      if (
                        (await confirmAction({
                          title: `Theme „${chosen.name}“ löschen?`,
                          confirmLabel: "Löschen",
                        })) &&
                        (await changeTheme({ kind: "delete", id: chosen.id }))
                      ) {
                        setPicked(null);
                        setNotice({
                          tone: "success",
                          message: "Theme gelöscht.",
                        });
                      }
                    }}
                  >
                    Löschen
                  </Button>
                </>
              )}
              <Button size="sm" onClick={() => exportTheme(chosen)}>
                Exportieren
              </Button>
            </div>
          )}
        </div>
      </div>
      {previewing ? (
        <div className="themes-preview-bar" role="region" aria-label="Vorschau">
          <span>
            <strong>„{themeName(selected)}“</strong> wird angezeigt
          </span>
          <div className="themes-actions">
            <Button variant="primary" disabled={blocked} onClick={keep}>
              Behalten
            </Button>
            <Button variant="ghost" onClick={revert}>
              Zurück zu {themeName(active)}
            </Button>
          </div>
        </div>
      ) : null}
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
