// `/`: jump into the first server, or explain how to get one.

import { Redirect } from "../components/Redirect.tsx";
import { useState } from "react";

import { useSession } from "../auth/session.ts";
import { CreateServerDialog } from "../components/ServerDialogs.tsx";
import { useLastChannel } from "../servers/lastChannel.ts";
import { useServers } from "../servers/queries.ts";

export function WorkspaceIndexPage() {
  const user = useSession((state) => state.user);
  const { data: servers, isPending, isError } = useServers();
  const lastByServer = useLastChannel((s) => s.byServer);
  const [creating, setCreating] = useState(false);

  const first = servers?.[0];
  if (first) {
    const channelId = lastByServer[first.id];
    return channelId ? (
      <Redirect
        to="/s/$serverId/c/$channelId"
        params={{ serverId: first.id, channelId }}
      />
    ) : (
      <Redirect to="/s/$serverId" params={{ serverId: first.id }} />
    );
  }

  if (isPending && !isError) {
    // First load only; afterwards the list is cached.
    return null;
  }

  return (
    <section className="mx-auto flex max-w-md flex-col gap-4 overflow-y-auto px-6 py-16">
      <h1 className="text-3xl font-semibold tracking-tight">
        {user ? `Hallo ${user.name}` : "Hallo"}
      </h1>
      <p className="text-neutral-600">
        {isError
          ? "Deine Server konnten gerade nicht geladen werden."
          : "Du bist noch in keinem Server. Erstelle einen — oder öffne einen Einladungslink, den dir jemand geschickt hat."}
      </p>
      <div>
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700"
        >
          Server erstellen
        </button>
      </div>
      <CreateServerDialog open={creating} onClose={() => setCreating(false)} />
    </section>
  );
}
