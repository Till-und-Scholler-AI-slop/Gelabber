// Who can be mentioned in the message pane (issue #241), for its rows and
// its composer.
import { createContext, useContext } from "react";

import type { MentionMember } from "../messages/mentions.ts";

type Mentions = {
  members: readonly MentionMember[];
  me: string | undefined;
  /** `@everyone` / `@here` are offered (server channels). */
  keywords: boolean;
};

export const MentionContext = createContext<Mentions>({
  members: [],
  me: undefined,
  keywords: false,
});

export function useMentions(): Mentions {
  return useContext(MentionContext);
}
