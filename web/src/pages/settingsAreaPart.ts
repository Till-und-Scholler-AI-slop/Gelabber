// The forms of a personal settings area are a chunk of their own; the
// settings header and navigation show at once (see UserSettingsPage).

import { lazyPart } from "../components/lazyPart.tsx";
import { translate } from "../i18n/catalog.ts";
import { settings } from "../i18n/catalogs/settings.ts";

export const SettingsAreaPart = lazyPart(
  () =>
    import("./SettingsAreaContent.tsx").then(
      (module) => module.SettingsAreaContent,
    ),
  // Taken in the language shown when the module loads: lazyPart wants a string.
  translate(settings, "areaPart.failed"),
);
