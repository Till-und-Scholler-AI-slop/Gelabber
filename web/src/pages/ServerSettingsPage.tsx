// `/s/$serverId/settings`: name, member permissions, invites, members,
// leave/delete. Open to every member; the editing sections need
// `manage_server`, deletion the owner.

import { useNavigate, useParams } from "@tanstack/react-router";
import { useState, type FormEvent, type ReactNode } from "react";

import { useSession } from "../auth/session.ts";
import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Avatar } from "../components/Avatar.tsx";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { GhostButton } from "../components/Modal.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { notify } from "../components/toasts.ts";
import { inviteUrl } from "../servers/api.ts";
import { copyText } from "../servers/clipboard.ts";
import { useLastChannel } from "../servers/lastChannel.ts";
import {
  PERMISSIONS,
  can,
  normalisePermissions,
} from "../servers/permissions.ts";
import {
  useCreateInvite,
  useDeleteServer,
  useInvites,
  useLeaveServer,
  useRevokeInvite,
  useServer,
  useUpdateServer,
} from "../servers/queries.ts";
import { validateName } from "../servers/rules.ts";
import type { Invite, Permission, ServerDetail } from "../servers/types.ts";

export function ServerSettingsPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId/settings" });
  const { data: server } = useServer(serverId);
  if (!server) return null;
  return <Settings key={server.id} server={server} />;
}

function Settings({ server }: { server: ServerDetail }) {
  const manage = can(server, "manage_server");
  return (
    <>
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-200 bg-white px-4">
        <h1 className="truncate font-semibold tracking-tight">
          Einstellungen · {server.name}
        </h1>
      </header>
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-10 px-6 py-8">
        {manage ? <NameSection server={server} /> : null}
        {manage ? <PermissionsSection server={server} /> : null}
        {manage ? <InvitesSection server={server} /> : null}
        <MembersSection server={server} />
        <DangerSection server={server} />
      </div>
    </>
  );
}

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-4">
      <div>
        <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
        {hint ? <p className="mt-1 text-sm text-neutral-600">{hint}</p> : null}
      </div>
      {children}
    </section>
  );
}

function NameSection({ server }: { server: ServerDetail }) {
  const [name, setName] = useState(server.name);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const update = useUpdateServer(server.id);
  const dirty = name.trim() !== server.name;

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateName(name);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    update.mutate({ name }, { onError: fromError });
  };

  return (
    <Section title="Name">
      <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4">
        <Field
          id="server-name"
          label="Servername"
          type="text"
          maxLength={100}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            clearField("name");
          }}
          error={errors.fields.name}
        />
        <FormError message={errors.form} />
        <div>
          <SubmitButton pending={false} pendingLabel="" disabled={!dirty}>
            Speichern
          </SubmitButton>
        </div>
      </form>
    </Section>
  );
}

/** Toggling a checkbox saves immediately (optimistic, rolled back on error). */
function PermissionsSection({ server }: { server: ServerDetail }) {
  const update = useUpdateServer(server.id);
  const current = new Set<Permission>(server.member_permissions);

  const toggle = (permission: Permission) => {
    const next = new Set(current);
    if (next.has(permission)) next.delete(permission);
    else next.add(permission);
    update.mutate({ member_permissions: normalisePermissions(next) });
  };

  return (
    <Section
      title="Rechte für Mitglieder"
      hint="Gilt für alle Mitglieder außer dem Owner, der immer alles darf. Änderungen greifen sofort."
    >
      <ul className="divide-y divide-neutral-200 rounded-xl border border-neutral-200 bg-white">
        {PERMISSIONS.map((permission) => {
          const checked = current.has(permission.id);
          return (
            <li key={permission.id}>
              <label className="flex cursor-pointer items-start gap-3 px-4 py-3">
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => toggle(permission.id)}
                  className="mt-1 size-4 accent-neutral-900"
                />
                <span className="flex flex-col">
                  <span className="text-sm font-medium text-neutral-900">
                    {permission.label}
                  </span>
                  <span className="text-sm text-neutral-500">
                    {permission.hint}
                  </span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </Section>
  );
}

function InvitesSection({ server }: { server: ServerDetail }) {
  const { data: invites, isPending } = useInvites(server.id, true);
  const create = useCreateInvite(server.id);
  const revoke = useRevokeInvite(server.id);

  return (
    <Section
      title="Einladungen"
      hint="Jedes Mitglied kann Links erstellen; hier siehst und widerrufst du alle aktiven."
    >
      <div>
        <button
          type="button"
          onClick={() =>
            create.mutate(
              {},
              {
                onSuccess: async (invite) => {
                  if (await copyText(inviteUrl(invite.code)))
                    notify("Einladungslink kopiert.");
                },
              },
            )
          }
          className="rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700"
        >
          Neuen Link erstellen
        </button>
      </div>
      {isPending ? (
        <p className="text-sm text-neutral-500">Lade Einladungen…</p>
      ) : !invites || invites.length === 0 ? (
        <p className="text-sm text-neutral-500">Keine aktiven Einladungen.</p>
      ) : (
        <ul className="divide-y divide-neutral-200 rounded-xl border border-neutral-200 bg-white">
          {invites.map((invite) => (
            <InviteRow
              key={invite.code}
              invite={invite}
              onRevoke={() => revoke.mutate(invite.code)}
            />
          ))}
        </ul>
      )}
    </Section>
  );
}

function InviteRow({
  invite,
  onRevoke,
}: {
  invite: Invite;
  onRevoke: () => void;
}) {
  const url = inviteUrl(invite.code);
  const uses =
    invite.max_uses === null
      ? `${invite.uses}× genutzt`
      : `${invite.uses}/${invite.max_uses} genutzt`;
  const expiry = invite.expires_at
    ? `läuft ab ${new Date(invite.expires_at).toLocaleString("de-DE", {
        dateStyle: "medium",
        timeStyle: "short",
      })}`
    : "läuft nie ab";
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="truncate font-mono text-sm">{url}</p>
        <p className="text-xs text-neutral-500">
          {uses} · {expiry}
        </p>
      </div>
      <GhostButton
        onClick={async () => {
          if (await copyText(url)) notify("Einladungslink kopiert.");
        }}
      >
        Kopieren
      </GhostButton>
      <GhostButton tone="danger" onClick={onRevoke}>
        Widerrufen
      </GhostButton>
    </li>
  );
}

function MembersSection({ server }: { server: ServerDetail }) {
  return (
    <Section title={`Mitglieder (${server.members.length})`}>
      <ul className="divide-y divide-neutral-200 rounded-xl border border-neutral-200 bg-white">
        {server.members.map((member) => (
          <li
            key={member.user_id}
            className="flex items-center gap-3 px-4 py-2.5"
          >
            <Avatar name={member.name} url={member.avatar_url} />
            <span className="min-w-0 flex-1 truncate text-sm font-medium">
              {member.name}
            </span>
            <span
              className={[
                "rounded-full px-2 py-0.5 text-xs font-medium",
                member.role === "owner"
                  ? "bg-amber-100 text-amber-800"
                  : "bg-neutral-100 text-neutral-600",
              ].join(" ")}
            >
              {member.role === "owner" ? "Owner" : "Mitglied"}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function DangerSection({ server }: { server: ServerDetail }) {
  const navigate = useNavigate();
  const user = useSession((s) => s.user);
  const forget = useLastChannel((s) => s.forget);
  const remove = useDeleteServer();
  const leave = useLeaveServer();
  const owner = server.role === "owner";

  if (!user) return null;

  const go = () => {
    forget(server.id);
    void navigate({ to: "/", replace: true });
  };

  return (
    <Section
      title={owner ? "Server löschen" : "Server verlassen"}
      hint={
        owner
          ? "Löscht Kanäle, Kategorien, Einladungen und Mitgliedschaften. Es gibt kein Zurück."
          : "Du kannst später mit einem neuen Einladungslink wieder beitreten."
      }
    >
      <div>
        <button
          type="button"
          onClick={() => {
            if (owner) {
              if (!window.confirm(`Server „${server.name}“ endgültig löschen?`))
                return;
              remove.mutate(server.id);
            } else {
              if (!window.confirm(`Server „${server.name}“ verlassen?`)) return;
              leave.mutate(server.id);
            }
            // Optimistic: the server is already gone from the rail.
            go();
          }}
          className="rounded-lg border border-red-300 px-4 py-2 text-sm font-medium text-red-700 transition hover:bg-red-50"
        >
          {owner ? "Server löschen" : "Server verlassen"}
        </button>
      </div>
    </Section>
  );
}
