import { translate } from "../../i18n/catalog.ts";
import { shell } from "../../i18n/catalogs/shell.ts";

/** What the show/hide toggle renders for a given state. */
export function passwordToggle(shown: boolean) {
  return {
    type: shown ? "text" : "password",
    pressed: shown,
    title: translate(shell, shown ? "password.hide" : "password.show"),
  } as const;
}
