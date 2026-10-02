import { useEffect } from "react";
import { useSession } from "../auth/session.ts";
import { applyDefinition } from "./theme.ts";
import { builtinTheme } from "./presets.ts";
import { bindThemeAccount, syncThemes, useAccountThemes } from "./account.ts";

export function ThemeController() {
  const sessionStatus = useSession((s) => s.status);
  const owner = useAccountThemes((s) => s.owner);
  const user = useSession((s) => s.user?.id ?? null);
  const doc = useAccountThemes((s) => s.doc);
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
    const apply = () => {
      const id =
        doc.active === "system"
          ? media.matches
            ? "dark"
            : "light"
          : doc.active;
      applyDefinition(
        doc.customThemes.find((t) => t.id === id) ?? builtinTheme(id),
        doc.active === "system",
      );
    };
    apply();
    if (doc.active !== "system") return;
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [doc, owner, user, sessionStatus]);
  return null;
}
