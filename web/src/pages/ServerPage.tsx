import { useParams } from "@tanstack/react-router";
import { LoadError } from "../components/LoadError.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
import { useServer } from "../servers/queries.ts";
import { CommunityOverview } from "./CommunityOverview.tsx";

export function ServerPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId" });
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  if (!server) {
    return error ? (
      <div className="p-6">
        <LoadError
          message="Der Server konnte nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      </div>
    ) : (
      <MainSkeleton kind="overview" />
    );
  }
  return <CommunityOverview key={server.id} server={server} />;
}
