import { useEffect, useRef } from "react";
import { useSession } from "../auth/session.ts";
import { useReducedMotion } from "../interface/useReducedMotion.ts";
import { applyDefinition } from "./theme.ts";
import { builtinTheme } from "./presets.ts";
import { bindThemeAccount, syncThemes, useAccountThemes } from "./account.ts";
import { displayedThemeId, useThemePreview } from "./preview.ts";

export function ThemeController() {
  const sessionStatus = useSession((s) => s.status);
  const owner = useAccountThemes((s) => s.owner);
  const user = useSession((s) => s.user?.id ?? null);
  const doc = useAccountThemes((s) => s.doc);
  const preview = useThemePreview((s) => s.id);
  const reduced = useReducedMotion();
  // The theme last put on the page: another one crossfades in.
  const appliedId = useRef<string | null>(null);
  useEffect(() => {
    if (sessionStatus === "unknown") return;
    bindThemeAccount(user);
    void syncThemes();
    const refresh = () => {
      if (document.visibilityState === "visible") void syncThemes();
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [user, sessionStatus]);
  useEffect(() => {
    if (sessionStatus === "unknown" || owner !== user) return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const shown = displayedThemeId(doc.active, preview);
    const apply = (fromSystem = false) => {
      const id =
        shown === "system" ? (media.matches ? "dark" : "light") : shown;
      const put = () =>
        applyDefinition(
          doc.customThemes.find((t) => t.id === id) ?? builtinTheme(id),
          shown === "system",
          { remember: preview === null },
        );
      // A different theme (picked, tried on, the system's turn) crossfades
      // the page where the browser can; the first paint and edits to the
      // theme shown apply at once. The system's turn does not hold up an
      // open dialog: the page takes no clicks while it crossfades.
      const switched = appliedId.current !== null && appliedId.current !== id;
      appliedId.current = id;
      const busy =
        fromSystem && document.querySelector("dialog[open]") !== null;
      if (switched && !reduced && !busy && document.startViewTransition)
        document.startViewTransition(put);
      else put();
    };
    apply();
    if (shown !== "system") return;
    const onSystem = () => apply(true);
    media.addEventListener("change", onSystem);
    return () => media.removeEventListener("change", onSystem);
  }, [doc, preview, owner, user, sessionStatus, reduced]);
  return null;
}
