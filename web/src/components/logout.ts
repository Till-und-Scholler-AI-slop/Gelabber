import { logout } from "../auth/session.ts";
import { leaveVoice, stopWatching, useVoice } from "../voice/session.ts";
import { confirmAction } from "./confirm.ts";

/** Whether signing out would end a call or a stream being watched. */
export function logoutEndsCall(voice: {
  status: string;
  watching: boolean;
}): boolean {
  return voice.status === "joined" || voice.watching;
}

/**
 * Signs this tab out, after asking when that would end a call. Resolves
 * whether it signed out; the caller then leaves for the login page.
 */
export async function logOut(): Promise<boolean> {
  if (
    logoutEndsCall(useVoice.getState()) &&
    !(await confirmAction({
      title: "Abmelden und Anruf beenden?",
      message: "Du verlässt dabei den Sprachraum, in dem du gerade bist.",
      confirmLabel: "Abmelden",
    }))
  )
    return false;
  // Store flips first, so the shell and guards react before the request
  // even leaves.
  leaveVoice();
  stopWatching();
  void logout();
  return true;
}
