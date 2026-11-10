// Who can be mentioned in the message pane (issue #241), for its rows and
// its composer.
import { createContext, useContext } from "react";

import type { MentionMember } from "../messages/mentions.ts";

type Mentions = {
  members: readonly MentionMember[];
  /** Who the `@…` suggestions offer: `members`, or in a channel limited
   * to roles only those who can read it. */
  mentionable: readonly MentionMember[];
  me: string | undefined;
  /** `@everyone` / `@here` are offered (server channels). */
  keywords: boolean;
};

export const MentionContext = createContext<Mentions>({
  members: [],
  mentionable: [],
  me: undefined,
  keywords: false,
});

export function useMentions(): Mentions {
  return useContext(MentionContext);
}
