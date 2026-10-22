// Contexts and handles MessagePane shares with its rows and composer.

import { createContext } from "react";
import { type SendAttempt } from "../messages/pending.ts";

// Shows an answered message: in the list when it is loaded, else in its
// context from the server.
export const JumpTo = createContext<(id: string) => void>(() => {});

/** What a row that was not sent offers besides "Verwerfen". */
export type AttemptActions = {
  retry: (attempt: SendAttempt) => void;
  /** Back into the composer, to change it before sending again. */
  edit: (attempt: SendAttempt) => void;
  canSendFiles: boolean;
};
export const AttemptActionsContext = createContext<AttemptActions | null>(null);

export type ComposerHandle = { takeBack: (attempt: SendAttempt) => void };
