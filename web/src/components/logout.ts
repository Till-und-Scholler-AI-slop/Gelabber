import { logout } from "../auth/session.ts";
import { leaveVoice, useVoice } from "../voice/session.ts";
import { confirmAction } from "./confirm.ts";

/** Whether signing out would end a call. */
export function logoutEndsCall(voice: { status: string }): boolean {
  return voice.status === "joined";
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
      message: "Du verlässt dabei den Sprachkanal.",
      confirmLabel: "Abmelden",
    }))
  )
    return false;
  // Store flips first, so the shell and guards react before the request
  // even leaves.
  leaveVoice();
  void logout();
  return true;
}
