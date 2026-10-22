// The forms of a personal settings area are a chunk of their own; the
// settings header and navigation show at once (see UserSettingsPage).

import { lazyPart } from "../components/lazyPart.tsx";

export const SettingsAreaPart = lazyPart(
  () =>
    import("./SettingsAreaContent.tsx").then(
      (module) => module.SettingsAreaContent,
    ),
  "Dieser Bereich konnte nicht geladen werden.",
);
