import { confirmAction } from "../components/confirm.ts";

export function confirmDiscardThemeDraft(): Promise<boolean> {
  return confirmAction({
    title: "Ungespeicherten Theme-Entwurf verwerfen?",
    message: "Deine Änderungen an diesem Theme gehen verloren.",
    confirmLabel: "Verwerfen",
  });
}
