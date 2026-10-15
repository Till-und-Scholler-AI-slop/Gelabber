import { confirmAction } from "../components/confirm.ts";

export function confirmDiscardThemeDraft(): Promise<boolean> {
  return confirmAction({
    title: "Ungespeicherten Theme-Entwurf verwerfen?",
    message: "Deine Änderungen an diesem Theme gehen verloren.",
    confirmLabel: "Verwerfen",
  });
}

/**
 * The route blocker's answer while the theme editor is open: `true` keeps
 * the user here. A clean editor never asks; a dirty one asks and stays
 * unless the user discards the draft.
 */
export async function blockThemeLeave(dirty: boolean): Promise<boolean> {
  return dirty && !(await confirmDiscardThemeDraft());
}
