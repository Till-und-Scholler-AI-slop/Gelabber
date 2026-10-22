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
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { SkeletonCard } from "../components/ui/Skeleton.tsx";
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

  if (preview.isError || (joinFailure && DEAD_INVITE.includes(joinFailure))) {
    const failure = joinFailure ?? errorCode(preview.error);
    return DEAD_INVITE.includes(failure) ? (
      <DeadInvite code={failure} />
    ) : (
      <section className="lr-auth-card">
        <h1>Einladung konnte nicht geladen werden</h1>
        <p role="alert" className="auth-lead">
          {errorMessage(failure)}
        </p>
        <div className="auth-actions">
          <Button
            variant="primary"
            size="lg"
            pending={preview.isFetching}
            pendingLabel="Wird geladen…"
            onClick={() => void preview.refetch()}
          >
            Erneut versuchen
          </Button>
          <Link
            to="/"
            className={buttonClass({ variant: "ghost", size: "lg" })}
          >
            Zu deinen Servern
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
            Zum Server
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
              pendingLabel="Beitreten…"
            >
              Beitreten
            </SubmitButton>
          </form>
        )}
        <Link to="/" className={buttonClass({ variant: "ghost", size: "lg" })}>
          Nicht jetzt
        </Link>
      </div>
    </section>
  );
}

function InviteServer({ preview }: { preview: InvitePreview }) {
  const { server, member } = preview;
  const people =
    server.member_count === 1
      ? "1 Mitglied"
      : `${server.member_count} Mitglieder`;
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
        {initials(server.name)}
      </span>
      <p>
        {member ? "Du bist schon Mitglied von" : "Du wurdest eingeladen zu"}
      </p>
      <h1 id="invite-server-name">{server.name}</h1>
      <p>{expiry ? `${people} · ${expiry}` : people}</p>
    </div>
  );
}

function DeadInvite({ code }: { code: ApiErrorCode }) {
  return (
    <section className="lr-auth-card">
      <h1>{code === "banned" ? "Kein Zutritt" : "Einladung nicht nutzbar"}</h1>
      <div role="alert" className="auth-result">
        <p>
          {code === "not_found"
            ? "Diesen Einladungslink gibt es nicht. Vielleicht wurde er widerrufen oder falsch kopiert."
            : errorMessage(code)}
        </p>
        {code === "banned" ? null : (
          <p className="auth-quiet">
            Bitte die Person, die dich eingeladen hat, um einen neuen Link.
          </p>
        )}
      </div>
      <div className="auth-actions">
        <Link to="/" className={buttonClass({ size: "lg" })}>
          Zu deinen Servern
        </Link>
      </div>
    </section>
  );
}
