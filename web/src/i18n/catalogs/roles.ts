import { defineCatalog } from "../catalog.ts";

// The fixed roles and the access summary of a channel (servers/roles.ts),
// and the lock on a limited channel.
export const roles = defineCatalog(
  "roles",
  {
    owner: "Inhaber",
    base: "Alle Mitglieder",
    ownerOnly: "Nur Inhaber",
    limitedLabel: "Nur für bestimmte Rollen",
  },
  {
    owner: "Owner",
    base: "All members",
    ownerOnly: "Owner only",
    limitedLabel: "Limited to certain roles",
  },
);
