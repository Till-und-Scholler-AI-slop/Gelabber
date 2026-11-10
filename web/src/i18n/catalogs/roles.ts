import { defineCatalog } from "../catalog.ts";

// The fixed roles and the access summary of a channel (servers/roles.ts).
export const roles = defineCatalog(
  "roles",
  {
    owner: "Inhaber",
    base: "Alle Mitglieder",
    ownerOnly: "Nur Inhaber",
  },
  {
    owner: "Owner",
    base: "All members",
    ownerOnly: "Owner only",
  },
);
