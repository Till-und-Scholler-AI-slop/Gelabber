import { useParams } from "@tanstack/react-router";
import { useServer } from "../servers/queries.ts";
import { CommunityOverview } from "./CommunityOverview.tsx";

export function ServerPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId" });
  const { data: server, error, refetch } = useServer(serverId);
  if (!server) {
    return (
      <div
        className="community-overview__empty"
        role={error ? "alert" : "status"}
      >
        <p>
          {error
            ? "Deine Community konnte gerade nicht geladen werden."
            : "Deine Community wird geladen…"}
        </p>
        {error ? (
          <button type="button" onClick={() => void refetch()}>
            Erneut versuchen
          </button>
        ) : null}
      </div>
    );
  }
  return <CommunityOverview key={server.id} server={server} />;
}
