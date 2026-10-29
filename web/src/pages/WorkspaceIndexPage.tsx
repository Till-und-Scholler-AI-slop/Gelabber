// `/`: reopen the server or DM the user was in last (else the first
// server), or welcome someone who has none yet.

import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { useUserId } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { Redirect } from "../components/Redirect.tsx";
import {
  CreateServerDialog,
  JoinServerForm,
} from "../components/ServerDialogs.tsx";
import { Button } from "../components/ui/Button.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
import { useDms } from "../dms/queries.ts";
import { InstallDisclosure } from "../pwa/InstallApp.tsx";
import { installHintUseful, useInstallation } from "../pwa/install.ts";
import { startPlace, useLastPlace } from "../servers/lastPlace.ts";
import { useServers } from "../servers/queries.ts";
import "./welcome.css";

export function WorkspaceIndexPage() {
  const user = useSession((state) => state.user);
  const userId = useUserId();
  const remembered = useLastPlace((state) => state.byUser[userId]);
  const {
    data: servers,
    isPending,
    isError,
    isFetching,
    refetch,
  } = useServers();
  const dms = useDms();
  const [creating, setCreating] = useState(false);
  const installHint = useInstallation(installHintUseful);

  const place = startPlace(
    remembered,
    servers,
    dms.data,
    !dms.isPending || dms.isError,
  );
  if (place?.kind === "server") {
    return <Redirect to="/s/$serverId" params={{ serverId: place.id }} />;
  }
  if (place?.kind === "dm") {
    return <Redirect to="/d/$channelId" params={{ channelId: place.id }} />;
  }

  if ((isPending || place === undefined) && !isError) {
    // First load only; afterwards the list is cached.
    return <MainSkeleton kind="overview" />;
  }

  return (
    <section className="welcome">
      <h1 className="welcome__title">
        {user ? `Willkommen, ${user.name}` : "Willkommen"}
      </h1>
      {isError ? (
        // Its own element, mounted with the failure: a role added to an
        // existing node together with its text is not reliably announced.
        <div className="welcome__error">
          <p key="error" role="alert">
            Die Server konnten nicht geladen werden.
          </p>
          <Button
            size="sm"
            pending={isFetching}
            pendingLabel="Wird geladen…"
            onClick={() => void refetch()}
          >
            Erneut versuchen
          </Button>
        </div>
      ) : null}
      <div className="welcome__choices">
        <article className="welcome-card">
          <h2>Eigenen Server erstellen</h2>
          <p>Mit #allgemein und einem Sprachkanal.</p>
          <Button
            variant="primary"
            size="lg"
            className="welcome-card__action"
            onClick={() => setCreating(true)}
          >
            Server erstellen
          </Button>
        </article>
        <article className="welcome-card">
          <h2>Server beitreten</h2>
          <JoinServerForm inline />
        </article>
      </div>
      <div className="welcome__next">
        {user && !user.avatar_url ? (
          <Link
            to="/settings/$area"
            params={{ area: "profil" }}
            className="welcome__next-link"
          >
            Profilbild festlegen ›
          </Link>
        ) : null}
        {installHint ? <InstallDisclosure /> : null}
      </div>
      <CreateServerDialog open={creating} onClose={() => setCreating(false)} />
    </section>
  );
}
