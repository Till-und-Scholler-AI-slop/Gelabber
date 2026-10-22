/** What the show/hide toggle renders for a given state. */
export function passwordToggle(shown: boolean) {
  return {
    type: shown ? "text" : "password",
    pressed: shown,
    title: shown ? "Passwort verbergen" : "Passwort anzeigen",
  } as const;
}
