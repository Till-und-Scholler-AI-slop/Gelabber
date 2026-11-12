import { create } from "zustand";
import { api, ApiError } from "../api/client.ts";
import { translate } from "../i18n/catalog.ts";
import { themes } from "../i18n/catalogs/themes.ts";
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
/** A sending mark older than this belongs to a tab that went away mid-save. */
const SENDING_STALE_MS = 30_000;
function validOperation(v: unknown): v is Operation {
  if (!v || typeof v !== "object" || !("kind" in v)) return false;
  return v.kind === "save"
    ? "theme" in v && isTheme(v.theme) && isCustomId(v.theme.id)
    : (v.kind === "apply" || v.kind === "delete") &&
        "id" in v &&
        typeof v.id === "string";
}
/** `sending`: when the PUT went out, so another tab does not adopt it while
 * it is in flight. */
function cache(sending = false): void {
  const s = useAccountThemes.getState();
  if (!s.owner) return;
  try {
    localStorage.setItem(
      key(s.owner),
      JSON.stringify({
        doc: s.doc,
        pending: s.pending,
        ...(sending ? { sending: Date.now() } : {}),
      }),
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
        // A save still marked as sending after a while never finished: the
        // tab closed or switched account, so this one takes the change over.
        const sending =
          "sending" in saved &&
          typeof saved.sending === "number" &&
          Date.now() - saved.sending < SENDING_STALE_MS;
        if ("pending" in saved && validOperation(saved.pending) && !sending)
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
    error: pending ? translate(themes, "error.notSynced") : "",
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
      throw new Error(translate(themes, "error.tooMany"));
    next.customThemes.push(op.theme);
    next.active = op.theme.id;
  } else if (op.kind === "delete") {
    next.customThemes = next.customThemes.filter((t) => t.id !== op.id);
    if (next.active === op.id) next.active = "dark";
  } else next = { ...next, active: op.id };
  if (!isThemeDocument(next, BUILTIN_IDS))
    throw new Error(translate(themes, "error.unavailable"));
  return next;
}
export async function syncThemes(): Promise<void> {
  const before = useAccountThemes.getState();
  if (!before.owner || busy) return;
  if (before.pending) {
    // An unsynced change (offline, or restored from the cache) goes out on
    // the next sync; a conflict waits for the user's choice.
    if (before.status === "error") await sendPending(true);
    return;
  }
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
      throw new Error(translate(themes, "error.serverFormat"));
    if (remote.revision === 0) {
      // Import the old device preference only for an account without theme settings.
      useAccountThemes.setState({
        doc: { ...remote, active: before.doc.active },
        status: "ready",
      });
      await change({ kind: "apply", id: before.doc.active }, true);
    } else {
      useAccountThemes.setState({ doc: remote, status: "ready", error: "" });
      cache();
    }
  } catch {
    if (stamp === generation && request === readId)
      useAccountThemes.setState({
        status: "error",
        error: translate(themes, "error.loadFailed"),
      });
  }
}
export function changeTheme(op: Operation): Promise<boolean> {
  return change(op, false);
}
/** `importing`: the first write of a fresh account, which another tab may
 * have done already. */
async function change(op: Operation, importing: boolean): Promise<boolean> {
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
    await sendPending(false, importing);
    return true; // Accepted locally; failed sync remains visible and retryable.
  } catch (error) {
    useAccountThemes.setState({
      error:
        error instanceof Error
          ? error.message
          : translate(themes, "error.saveFailed"),
    });
    return false;
  }
}
async function sendPending(refresh: boolean, importing = false): Promise<void> {
  const s = useAccountThemes.getState();
  if (!s.owner || !s.pending || busy) return;
  const stamp = generation;
  let reread = false;
  busy = true;
  readId++;
  useAccountThemes.setState({ status: "saving", error: "" });
  cache(true);
  try {
    let next = s.doc;
    if (refresh) {
      const remote = await api<ThemeDocument>("/me/themes");
      if (stamp !== generation) return;
      if (!isThemeDocument(remote, BUILTIN_IDS))
        throw new Error(translate(themes, "error.unknownFormat"));
      next = applyOperation(remote, s.pending);
    }
    const saved = await api<ThemeDocument>("/me/themes", {
      method: "PUT",
      body: next,
    });
    if (stamp !== generation) return;
    if (!isThemeDocument(saved, BUILTIN_IDS))
      throw new Error(translate(themes, "error.unknownFormat"));
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
    if (conflict && importing) {
      // Another tab or device set the account up first: take its state.
      reread = true;
      useAccountThemes.setState({ pending: null, status: "loading" });
    } else
      useAccountThemes.setState({
        status: conflict ? "conflict" : "error",
        error: conflict
          ? translate(themes, "error.conflict")
          : translate(themes, "error.offline"),
      });
    cache();
  } finally {
    if (stamp === generation) busy = false;
  }
  if (reread) await syncThemes();
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
