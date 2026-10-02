import { create } from "zustand";
import { api, ApiError } from "../api/client.ts";
import { BUILTIN_IDS } from "./presets.ts";
import {
  isThemeDocument,
  isTheme,
  isCustomId,
  MAX_THEMES,
  type ThemeDefinition,
  type ThemeDocument,
} from "./model.ts";
import { loadThemePreference } from "./theme.ts";

type Operation =
  | { kind: "apply"; id: string }
  | { kind: "save"; theme: ThemeDefinition }
  | { kind: "delete"; id: string };
type State = {
  owner: string | null;
  doc: ThemeDocument;
  pending: Operation | null;
  status: "loading" | "ready" | "saving" | "error" | "conflict";
  error: string;
};
const initial = (): ThemeDocument => ({
  version: 1,
  revision: 0,
  active: loadThemePreference(),
  customThemes: [],
});
export const useAccountThemes = create<State>(() => ({
  owner: null,
  doc: initial(),
  pending: null,
  status: "loading",
  error: "",
}));
let generation = 0;
let readId = 0;
let busy = false;
const key = (owner: string) => `gelabber.themes.v1:${owner}`;
function validOperation(v: unknown): v is Operation {
  if (!v || typeof v !== "object" || !("kind" in v)) return false;
  return v.kind === "save"
    ? "theme" in v && isTheme(v.theme) && isCustomId(v.theme.id)
    : (v.kind === "apply" || v.kind === "delete") &&
        "id" in v &&
        typeof v.id === "string";
}
function cache(): void {
  const s = useAccountThemes.getState();
  if (!s.owner) return;
  try {
    localStorage.setItem(
      key(s.owner),
      JSON.stringify({ doc: s.doc, pending: s.pending }),
    );
  } catch {
    /* Memory state still works. */
  }
}
export function bindThemeAccount(owner: string | null): void {
  if (useAccountThemes.getState().owner === owner) return;
  generation++;
  readId++;
  busy = false;
  let doc = initial();
  let pending: Operation | null = null;
  if (owner) {
    try {
      const saved: unknown = JSON.parse(
        localStorage.getItem(key(owner)) ?? "null",
      );
      if (
        saved &&
        typeof saved === "object" &&
        "doc" in saved &&
        isThemeDocument(saved.doc, BUILTIN_IDS)
      ) {
        doc = saved.doc;
        if ("pending" in saved && validOperation(saved.pending))
          pending = saved.pending;
      }
    } catch {
      /* Corrupt/blocked cache falls back to server. */
    }
  }
  useAccountThemes.setState({
    owner,
    doc,
    pending,
    status: pending ? "error" : "loading",
    error: pending ? "Noch nicht synchronisiert." : "",
  });
  if (!owner && typeof document !== "undefined") {
    try {
      localStorage.removeItem("gelabber.theme.paint");
    } catch {
      /* optional cache */
    }
  }
}
function applyOperation(doc: ThemeDocument, op: Operation): ThemeDocument {
  let next = { ...doc, customThemes: [...doc.customThemes] };
  if (op.kind === "save") {
    next.customThemes = next.customThemes.filter((t) => t.id !== op.theme.id);
    if (next.customThemes.length >= MAX_THEMES)
      throw new Error("Du kannst höchstens 50 eigene Themes speichern.");
    next.customThemes.push(op.theme);
    next.active = op.theme.id;
  } else if (op.kind === "delete") {
    next.customThemes = next.customThemes.filter((t) => t.id !== op.id);
    if (next.active === op.id) next.active = "dark";
  } else next = { ...next, active: op.id };
  if (!isThemeDocument(next, BUILTIN_IDS))
    throw new Error("Das ausgewählte Theme ist nicht mehr verfügbar.");
  return next;
}
export async function syncThemes(): Promise<void> {
  const before = useAccountThemes.getState();
  if (!before.owner || before.pending || busy) return;
  const stamp = generation;
  const request = ++readId;
  try {
    const remote = await api<ThemeDocument>("/me/themes");
    if (
      stamp !== generation ||
      request !== readId ||
      useAccountThemes.getState().pending
    )
      return;
    if (!isThemeDocument(remote, BUILTIN_IDS))
      throw new Error("Unbekanntes Theme-Format vom Server.");
    if (remote.revision === 0) {
      // Import the old device preference only for an account without theme settings.
      useAccountThemes.setState({
        doc: { ...remote, active: before.doc.active },
        status: "ready",
      });
      await changeTheme({ kind: "apply", id: before.doc.active });
    } else {
      useAccountThemes.setState({ doc: remote, status: "ready", error: "" });
      cache();
    }
  } catch {
    if (stamp === generation && request === readId)
      useAccountThemes.setState({
        status: "error",
        error: "Themes konnten nicht geladen werden. Bitte erneut versuchen.",
      });
  }
}
export async function changeTheme(op: Operation): Promise<boolean> {
  const s = useAccountThemes.getState();
  if (
    !s.owner ||
    busy ||
    s.pending ||
    s.status === "loading" ||
    s.status === "error"
  )
    return false;
  try {
    const next = applyOperation(s.doc, op);
    readId++;
    useAccountThemes.setState({ doc: next, pending: op });
    cache();
    await sendPending(false);
    return true; // Accepted locally; failed sync remains visible and retryable.
  } catch (error) {
    useAccountThemes.setState({
      error:
        error instanceof Error
          ? error.message
          : "Theme konnte nicht gespeichert werden.",
    });
    return false;
  }
}
async function sendPending(refresh: boolean): Promise<void> {
  const s = useAccountThemes.getState();
  if (!s.owner || !s.pending || busy) return;
  const stamp = generation;
  busy = true;
  readId++;
  useAccountThemes.setState({ status: "saving", error: "" });
  try {
    let next = s.doc;
    if (refresh) {
      const remote = await api<ThemeDocument>("/me/themes");
      if (stamp !== generation) return;
      if (!isThemeDocument(remote, BUILTIN_IDS))
        throw new Error("Unbekanntes Theme-Format.");
      next = applyOperation(remote, s.pending);
    }
    const saved = await api<ThemeDocument>("/me/themes", {
      method: "PUT",
      body: next,
    });
    if (stamp !== generation) return;
    if (!isThemeDocument(saved, BUILTIN_IDS))
      throw new Error("Unbekanntes Theme-Format.");
    useAccountThemes.setState({
      doc: saved,
      pending: null,
      status: "ready",
      error: "",
    });
    cache();
  } catch (error) {
    if (stamp !== generation) return;
    const conflict = error instanceof ApiError && error.status === 409;
    useAccountThemes.setState({
      status: conflict ? "conflict" : "error",
      error: conflict
        ? "Auf einem anderen Gerät geändert. Deine Änderung bleibt erhalten. Erneut anwenden übernimmt sie auf den aktuellen Stand."
        : "Noch nicht synchronisiert. Deine Änderung ist hier gespeichert.",
    });
    cache();
  } finally {
    if (stamp === generation) busy = false;
  }
}
export async function retryThemes(): Promise<void> {
  if (useAccountThemes.getState().pending) await sendPending(true);
  else await syncThemes();
}
export async function discardPendingTheme(): Promise<void> {
  if (busy) return;
  useAccountThemes.setState({ pending: null, status: "loading", error: "" });
  cache();
  await syncThemes();
}
