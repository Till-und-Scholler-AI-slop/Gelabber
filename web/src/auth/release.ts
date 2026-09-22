// Tear down everything that belongs to one signed-in user. Device and
// display preferences stay; they are not account data.

import { clearToasts } from "../components/toasts.ts";
import { clearMessageToasts } from "../messages/toasts.ts";
import { resetPendingMessages } from "../messages/pending.ts";
import { dropForeignUserQueries, setQueryScope } from "../queryClient.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { leaveVoice, stopWatching } from "../voice/session.ts";
import { resetVoiceRoster } from "../voice/roster.ts";
import { getGateway } from "../ws/client.ts";
import { resetLiveStores } from "../ws/live.ts";
import { nextScopeGeneration } from "./scope.ts";

/**
 * Run on explicit logout, a 401, and any other identity change. `nextUserId`
 * is the account that now owns the tab, or null when nobody is signed in.
 */
export function releaseUserScope(nextUserId: string | null): void {
  const generation = nextScopeGeneration();
  setQueryScope(nextUserId, generation);
  // Stop local media first so a still-open socket can send leave, then drop
  // topics and resume cursors so the next account cannot resume A's seq.
  leaveVoice();
  stopWatching();
  getGateway().resetSession();
  dropForeignUserQueries();
  resetPendingMessages();
  clearMessageToasts();
  clearToasts();
  resetLiveStores();
  resetVoiceRoster();
  useMediaSettings.getState().closeDialog();
}
