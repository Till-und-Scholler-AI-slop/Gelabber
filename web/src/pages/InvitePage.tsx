// `/invite/$code`: what an invite link opens. The route guard makes sure the
// visitor is signed in (login/register bounce back here), then one click
// joins and lands in the server.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";

import { ApiError } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { previewInvite } from "../servers/api.ts";
import { useJoinInvite } from "../servers/queries.ts";

export function InvitePage() {
  const { code } = useParams({ from: "/centered/invite/$code" });
  const navigate = useNavigate();
  const preview = useQuery({
    queryKey: ["invite", code],
    queryFn: ({ signal }) => previewInvite(code, signal),
    retry: false,
  });
  const join = useJoinInvite();

  if (preview.isPending) {
    return null;
  }

  if (preview.isError) {
    const code =
      preview.error instanceof ApiError ? preview.error.code : "internal";
    return (
      <section className="mx-auto max-w-sm">
        <h1 className="text-2xl font-semibold tracking-tight">
          {code === "banned" ? "Kein Zutritt" : "Einladung nicht nutzbar"}
        </h1>
        <p className="mt-2 text-neutral-600 dark:text-neutral-400">
          {code === "not_found"
            ? "Diesen Einladungslink gibt es nicht. Vielleicht wurde er widerrufen oder falsch kopiert."
            : errorMessage(code)}
        </p>
        <p className="mt-6 text-sm">
          <Link
            to="/"
            className="font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2"
          >
            Zu deinen Servern
          </Link>
        </p>
      </section>
    );
  }

  const { server, member } = preview.data;
  const people =
    server.member_count === 1
      ? "1 Mitglied"
      : `${server.member_count} Mitglieder`;

  return (
    <section className="mx-auto max-w-sm">
      <p className="text-sm text-neutral-600 dark:text-neutral-400">
        {member ? "Du bist schon Mitglied von" : "Du wurdest eingeladen zu"}
      </p>
      <h1 className="mt-1 text-3xl font-semibold tracking-tight">
        {server.name}
      </h1>
      <p className="mt-1 text-sm text-neutral-500 dark:text-neutral-400">
        {people}
      </p>

      <div className="mt-8 flex flex-col gap-4">
        {member ? (
          <Link
            to="/s/$serverId"
            params={{ serverId: server.id }}
            className="inline-flex justify-center rounded-lg bg-neutral-900 dark:bg-neutral-700 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700 dark:hover:bg-neutral-600"
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
            className="flex flex-col gap-4"
          >
            <FormError
              message={
                join.error
                  ? errorMessage(
                      join.error instanceof ApiError
                        ? join.error.code
                        : "internal",
                    )
                  : null
              }
            />
            <SubmitButton pending={join.isPending} pendingLabel="Beitreten…">
              Beitreten
            </SubmitButton>
          </form>
        )}
      </div>
    </section>
  );
}
