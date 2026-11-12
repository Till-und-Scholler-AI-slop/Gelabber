import { useParams } from "@tanstack/react-router";
import { LoadError } from "../components/LoadError.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
import { useT } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import { useServer } from "../servers/queries.ts";
import { CommunityOverview } from "./CommunityOverview.tsx";

export function ServerPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId" });
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  const t = useT(servers);
  if (!server) {
    return error ? (
      <div className="p-6">
        <LoadError
          message={t("load.server")}
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
