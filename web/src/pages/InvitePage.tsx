// `/invite/$code`: what an invite link opens. The route guard makes sure the
// visitor is signed in (register/login bounce back here), then one click
// joins and lands in the server.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import type { CSSProperties } from "react";

import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import { errorMessage } from "../auth/rules.ts";
import { FormError } from "../components/FormError.tsx";
import { initials, tileTone } from "../components/initials.ts";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { SkeletonCard } from "../components/ui/Skeleton.tsx";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { common } from "../i18n/catalogs/common.ts";
import { previewInvite } from "../servers/api.ts";
import { inviteExpiry } from "../servers/inviteCode.ts";
import { serverKeys, useJoinInvite } from "../servers/queries.ts";
import type { InvitePreview } from "../servers/types.ts";

/** Answers that say the link itself is no good, not that loading failed. */
const DEAD_INVITE: readonly ApiErrorCode[] = [
  "not_found",
  "invite_invalid",
  "banned",
];

const errorCode = (error: unknown): ApiErrorCode =>
  error instanceof ApiError ? error.code : "internal";

export function InvitePage() {
  const t = useT(auth);
  const tc = useT(common);
  const { code } = useParams({ from: "/auth/invite/$code" });
  const navigate = useNavigate();
  const userId = useUserId();
  const generation = scopeGeneration();
  const preview = useQuery({
    queryKey: serverKeys.invite(userId, generation, code),
    queryFn: ({ signal }) => previewInvite(code, signal),
    enabled: userId.length > 0,
    retry: false,
  });
  const join = useJoinInvite();
  const joinFailure = join.error ? errorCode(join.error) : null;

  if (preview.isPending) {
    return <SkeletonCard className="invite-skeleton" />;
  }

  // A failed background refetch keeps the invite that is shown.
  if (!preview.data || (joinFailure && DEAD_INVITE.includes(joinFailure))) {
    const failure = joinFailure ?? errorCode(preview.error);
    return DEAD_INVITE.includes(failure) ? (
      <DeadInvite code={failure} />
    ) : (
      <section className="lr-auth-card">
        <h1>{t("invite.loadFailed")}</h1>
        <p role="alert" className="auth-lead">
          {errorMessage(failure)}
        </p>
        <div className="auth-actions">
          <Button
            variant="primary"
            size="lg"
            pending={preview.isFetching}
            pendingLabel={tc("loading")}
            onClick={() => void preview.refetch()}
          >
            {tc("tryAgain")}
          </Button>
          <Link
            to="/"
            className={buttonClass({ variant: "ghost", size: "lg" })}
          >
            {t("invite.toServers")}
          </Link>
        </div>
      </section>
    );
  }

  const { server, member } = preview.data;

  return (
    <section className="lr-auth-card" aria-labelledby="invite-server-name">
      <InviteServer preview={preview.data} />
      <div className="auth-actions">
        {member ? (
          <Link
            to="/s/$serverId"
            params={{ serverId: server.id }}
            className={buttonClass({ variant: "primary", size: "lg" })}
          >
            {t("invite.toServer")}
          </Link>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              join.mutate(code, {
                onSuccess: (server) =>
                  void navigate({
                    to: "/s/$serverId",
                    params: { serverId: server.id },
                    replace: true,
                  }),
              });
            }}
            className="flex flex-col gap-3"
          >
            <FormError
              message={joinFailure ? errorMessage(joinFailure) : null}
            />
            <SubmitButton
              size="lg"
              pending={join.isPending}
              pendingLabel={t("invite.joining")}
            >
              {t("invite.join")}
            </SubmitButton>
          </form>
        )}
        <Link to="/" className={buttonClass({ variant: "ghost", size: "lg" })}>
          {tc("cancel")}
        </Link>
      </div>
    </section>
  );
}

function InviteServer({ preview }: { preview: InvitePreview }) {
  const t = useT(auth);
  const { server, member } = preview;
  const people = t("invite.members", { count: server.member_count });
  const expiry = member ? null : inviteExpiry(preview.expires_at);
  return (
    <div className="invite-server">
      <span
        aria-hidden="true"
        className="invite-tile"
        style={
          {
            "--lr-tile": `var(--lr-tile-${tileTone(server.id)})`,
          } as CSSProperties
        }
      >
        <ServerIconFace
          url={server.icon_url}
          fallback={initials(server.name)}
        />
      </span>
      <p>{member ? t("invite.alreadyMember") : t("invite.invitedTo")}</p>
      <h1 id="invite-server-name">{server.name}</h1>
      <p>{expiry ? `${people} · ${expiry}` : people}</p>
    </div>
  );
}

function DeadInvite({ code }: { code: ApiErrorCode }) {
  const t = useT(auth);
  return (
    <section className="lr-auth-card">
      <h1>
        {code === "banned" ? t("invite.bannedTitle") : t("invite.deadTitle")}
      </h1>
      <div role="alert" className="auth-result">
        <p>
          {code === "not_found" ? t("invite.notFound") : errorMessage(code)}
        </p>
        {code === "banned" ? null : (
          <p className="auth-quiet">{t("invite.askForNew")}</p>
        )}
      </div>
      <div className="auth-actions">
        <Link to="/" className={buttonClass({ size: "lg" })}>
          {t("invite.toServers")}
        </Link>
      </div>
    </section>
  );
}
