import { confirmAction } from "../components/confirm.ts";
import { translate } from "../i18n/catalog.ts";
import { themes } from "../i18n/catalogs/themes.ts";

export function confirmDiscardThemeDraft(): Promise<boolean> {
  return confirmAction({
    title: translate(themes, "draft.discardTitle"),
    message: translate(themes, "draft.discardMessage"),
    confirmLabel: translate(themes, "draft.discard"),
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
