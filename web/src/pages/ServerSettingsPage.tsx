// `/s/$serverId/settings`: name, member permissions, invites, members,
// bans, leave/delete. Open to every member; the editing sections need
// `manage_server`, deletion the owner.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import {
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type FormEvent,
  type ReactNode,
} from "react";

import { useUserId } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { ApiError } from "../api/client.ts";
import {
  AvatarImageError,
  avatarImageMessage,
  prepareAvatar,
} from "../auth/avatarImage.ts";
import { errorMessage, fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { PresenceAvatar } from "../components/PresenceAvatar.tsx";
import { Field } from "../components/Field.tsx";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { tileInitials, tileTint } from "../components/initials.ts";
import { confirmAction } from "../components/confirm.ts";
import { LoadError } from "../components/LoadError.tsx";
import { FormError } from "../components/FormError.tsx";
import { InviteDialog } from "../components/InviteDialog.tsx";
import { MemberActionsMenu } from "../components/MemberActions.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { Button } from "../components/ui/Button.tsx";
import { CopyButton } from "../components/ui/CopyButton.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { inviteUrl } from "../servers/api.ts";
import { copyText } from "../servers/clipboard.ts";
import { inviteExpiry, inviteUses } from "../servers/inviteText.ts";
import { useLastChannel } from "../servers/lastChannel.ts";
import {
  PERMISSIONS,
  can,
  normalisePermissions,
} from "../servers/permissions.ts";
import {
  readServer,
  useBans,
  useDeleteServer,
  useInvites,
  useLeaveServer,
  useRevokeInvite,
  useServer,
  useServerIcon,
  useUnbanMember,
  useUpdateServer,
} from "../servers/queries.ts";
import { validateName } from "../servers/rules.ts";
import type { Invite, Permission, ServerDetail } from "../servers/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import "./settings.css";

const NOT_SAVED = {
  tone: "error",
  message: "Nicht gespeichert – erneut versuchen",
} as const;

export function ServerSettingsPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId/settings" });
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  if (error && !server)
    return (
      <div className="p-6">
        <LoadError
          message="Die Servereinstellungen konnten gerade nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      </div>
    );
  if (!server) return null;
  return <Settings key={server.id} server={server} />;
}

type SectionId =
  | "allgemein"
  | "rechte"
  | "einladungen"
  | "mitglieder"
  | "sperren"
  | "gefahrenzone";

const SECTION_LABEL: Record<SectionId, string> = {
  allgemein: "Allgemein",
  rechte: "Rechte",
  einladungen: "Einladungen",
  mitglieder: "Mitglieder",
  sperren: "Sperren",
  gefahrenzone: "Gefahrenzone",
};

/** The sections this member sees, in page order. */
function serverSettingsSections(manage: boolean): SectionId[] {
  return manage
    ? [
        "allgemein",
        "rechte",
        "einladungen",
        "mitglieder",
        "sperren",
        "gefahrenzone",
      ]
    : ["mitglieder", "gefahrenzone"];
}

const sectionId = (id: SectionId) => `server-settings-${id}`;

function Settings({ server }: { server: ServerDetail }) {
  const manage = can(server, "manage_server");
  const sections = serverSettingsSections(manage);
  return (
    <div className="settings-screen">
      <header className="lr-channel-header settings-header">
        <h1>
          Servereinstellungen
          <span className="settings-header-context"> · {server.name}</span>
        </h1>
      </header>
      <div className="settings-scroll">
        <nav className="server-settings-nav" aria-label="Abschnitte">
          {sections.map((id) => (
            <a
              key={id}
              href={`#${sectionId(id)}`}
              onClick={(event) => {
                // Scroll inside the page; the address stays the server's.
                event.preventDefault();
                const heading = document.getElementById(sectionId(id));
                heading?.scrollIntoView({ block: "start" });
                heading?.focus({ preventScroll: true });
              }}
            >
              {SECTION_LABEL[id]}
            </a>
          ))}
        </nav>
        <div className="settings-area server-settings">
          {manage ? <NameSection server={server} /> : null}
          {manage ? <PermissionsSection server={server} /> : null}
          {manage ? <InvitesSection server={server} /> : null}
          <MembersSection server={server} />
          {manage ? <BansSection server={server} /> : null}
          <DangerSection server={server} />
        </div>
      </div>
    </div>
  );
}

function Section({
  id,
  title,
  hint,
  children,
}: {
  id: SectionId;
  title?: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section
      className="server-settings-section"
      aria-labelledby={sectionId(id)}
    >
      <header className="server-settings-section__heading">
        <h2 id={sectionId(id)} tabIndex={-1}>
          {title ?? SECTION_LABEL[id]}
        </h2>
        {hint ? <p>{hint}</p> : null}
      </header>
      {children}
    </section>
  );
}

function NameSection({ server }: { server: ServerDetail }) {
  const [name, setName] = useState(server.name);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const update = useUpdateServer(server.id);
  const status = useInlineStatus();
  const dirty = name.trim() !== server.name;

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateName(name);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    status.clear();
    update.mutate(
      { name },
      {
        onSuccess: () =>
          status.show({ tone: "success", message: "Gespeichert." }),
        onError: fromError,
      },
    );
  };

  return (
    <Section id="allgemein">
      <IconRow server={server} />
      <form onSubmit={onSubmit} noValidate className="settings-form">
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
        <div className="settings-actions">
          <SubmitButton
            pending={update.isPending}
            pendingLabel="Speichert…"
            disabled={!dirty}
          >
            Speichern
          </SubmitButton>
          <InlineStatus status={status.status} />
        </div>
      </form>
    </Section>
  );
}

/** Upload and removal apply at once, like the profile avatar. */
function IconRow({ server }: { server: ServerDetail }) {
  const picker = useRef<HTMLInputElement>(null);
  const icon = useServerIcon(server.id);
  const status = useInlineStatus();
  const [removing, setRemoving] = useState(false);
  const save = (image: Blob | null) => {
    status.clear();
    setRemoving(image === null);
    icon.mutate(image, {
      onSuccess: () =>
        status.show({ tone: "success", message: "Gespeichert." }),
      onError: (error) =>
        status.show({
          tone: "error",
          message:
            error instanceof AvatarImageError
              ? avatarImageMessage(error)
              : error instanceof ApiError
                ? errorMessage(error.code)
                : NOT_SAVED.message,
        }),
    });
  };
  const onPick = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file || icon.isPending) return;
    try {
      save(await prepareAvatar(file));
    } catch (error) {
      if (error instanceof AvatarImageError)
        status.show({ tone: "error", message: avatarImageMessage(error) });
      else throw error;
    }
  };
  return (
    <div className="settings-avatar-row">
      <span
        className="server-tile server-settings-icon"
        style={{ "--server-tile": tileTint(server.id) } as CSSProperties}
        aria-hidden="true"
      >
        <ServerIconFace
          url={server.icon_url}
          fallback={tileInitials(server.name)}
        />
      </span>
      <div className="settings-avatar-actions">
        <p className="settings-hint">
          Das Bild steht in der Serverleiste. Ohne Bild zeigen wir die Initialen
          des Servernamens.
        </p>
        <div className="settings-actions">
          <Button
            size="sm"
            onClick={() => picker.current?.click()}
            disabled={icon.isPending}
            pending={icon.isPending && !removing}
            pendingLabel="Lädt hoch…"
          >
            Bild hochladen
          </Button>
          {server.icon_url ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => save(null)}
              disabled={icon.isPending}
              pending={icon.isPending && removing}
              pendingLabel="Entfernt…"
            >
              Entfernen
            </Button>
          ) : null}
          <InlineStatus status={status.status} />
        </div>
        <input
          ref={picker}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          aria-label="Serverbild auswählen"
          onChange={(event) => void onPick(event)}
        />
      </div>
    </div>
  );
}

/** Each switch saves at once (optimistic, rolled back on error). */
function PermissionsSection({ server }: { server: ServerDetail }) {
  const client = useQueryClient();
  const userId = useUserId();
  const update = useUpdateServer(server.id);
  const status = useInlineStatus();
  const current = new Set<Permission>(server.member_permissions);

  const toggle = (permission: Permission) => {
    // Start from the cache, not from this render's props: a second click
    // before the first PATCH returns must build on the first click's
    // optimistic mask, or the later request would restore the flag.
    const latest = (userId && readServer(client, userId, server.id)) || server;
    const next = new Set<Permission>(latest.member_permissions);
    if (next.has(permission)) next.delete(permission);
    else next.add(permission);
    update.mutate(
      { member_permissions: normalisePermissions(next) },
      {
        onSuccess: () =>
          status.show({ tone: "success", message: "Gespeichert." }),
        onError: () => status.show(NOT_SAVED),
      },
    );
  };

  return (
    <Section
      id="rechte"
      title="Rechte für Mitglieder"
      hint="Gilt für alle Mitglieder außer dem Owner, der immer alles darf. Änderungen greifen sofort."
    >
      <div className="server-settings-list">
        {PERMISSIONS.map((permission) => (
          <Switch
            key={permission.id}
            className="settings-row"
            label={permission.label}
            description={permission.hint}
            checked={current.has(permission.id)}
            onChange={() => toggle(permission.id)}
          />
        ))}
      </div>
      <InlineStatus status={status.status} />
    </Section>
  );
}

function InvitesSection({ server }: { server: ServerDetail }) {
  const {
    data: invites,
    isPending,
    error,
    isFetching,
    refetch,
  } = useInvites(server.id, true);
  const revoke = useRevokeInvite(server.id);
  const [inviting, setInviting] = useState(false);
  const status = useInlineStatus();
  // When the page opened: "läuft in 6 Tagen ab" needs no live clock.
  const [now] = useState(Date.now);

  // The row's button says "Kopiert"; only a failure needs the line.
  const copy = async (invite: Invite): Promise<boolean> => {
    if (await copyText(inviteUrl(invite.code))) {
      status.clear();
      return true;
    }
    status.show({
      tone: "error",
      message: `Kopieren ging nicht. Der Code ist ${invite.code}.`,
    });
    return false;
  };

  return (
    <Section
      id="einladungen"
      hint="Jedes Mitglied kann Links erstellen; hier siehst und widerrufst du alle aktiven."
    >
      <div className="settings-actions">
        <Button onClick={() => setInviting(true)}>Neuen Link erstellen</Button>
        <InlineStatus status={status.status} />
      </div>
      <InviteDialog
        server={server}
        open={inviting}
        onClose={() => setInviting(false)}
      />
      {isPending ? (
        <p className="settings-hint">Lade Einladungen…</p>
      ) : error ? (
        <LoadError
          message="Die Einladungen konnten gerade nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      ) : !invites || invites.length === 0 ? (
        <p className="settings-hint">Keine aktiven Einladungen.</p>
      ) : (
        <ul className="server-settings-list">
          {invites.map((invite) => (
            <InviteRow
              key={invite.code}
              invite={invite}
              now={now}
              onCopy={() => copy(invite)}
              onRevoke={async () => {
                if (
                  await confirmAction({
                    title: "Link widerrufen?",
                    message:
                      "Wer ihn noch nicht benutzt hat, kommt damit nicht mehr rein. Mitglieder bleiben Mitglieder.",
                    confirmLabel: "Widerrufen",
                  })
                )
                  revoke.mutate(invite.code, {
                    onSuccess: () =>
                      status.show({
                        tone: "success",
                        message: "Link widerrufen.",
                      }),
                  });
              }}
            />
          ))}
        </ul>
      )}
    </Section>
  );
}

function InviteRow({
  invite,
  now,
  onCopy,
  onRevoke,
}: {
  invite: Invite;
  now: number;
  onCopy: () => Promise<boolean>;
  onRevoke: () => void;
}) {
  return (
    <li className="server-settings-row">
      <div className="server-settings-row__main">
        <code className="server-settings-code">{invite.code}</code>
        <span className="settings-hint">
          {inviteUses(invite)} · {inviteExpiry(invite.expires_at, now)}
        </span>
      </div>
      <div className="server-settings-row__actions">
        <CopyButton size="sm" onCopy={onCopy}>
          Kopieren
        </CopyButton>
        <Button size="sm" variant="danger-outline" onClick={onRevoke}>
          Widerrufen
        </Button>
      </div>
    </li>
  );
}

function MembersSection({ server }: { server: ServerDetail }) {
  const byServer = usePresenceStore((s) => s.byServer);
  return (
    <Section id="mitglieder" title={`Mitglieder (${server.members.length})`}>
      <ul className="server-settings-list">
        {server.members.map((member) => (
          <li key={member.user_id} className="server-settings-row">
            <PresenceAvatar
              name={member.name}
              url={member.avatar_url}
              status={presenceOf(byServer, server.id, member.user_id)}
            />
            <span className="server-settings-row__name">{member.name}</span>
            <Badge tone={member.role === "owner" ? "accent" : "neutral"}>
              {member.role === "owner" ? "Owner" : "Mitglied"}
            </Badge>
            <MemberActionsMenu server={server} member={member} />
          </li>
        ))}
      </ul>
    </Section>
  );
}

function BansSection({ server }: { server: ServerDetail }) {
  const {
    data: bans,
    isPending,
    error,
    isFetching,
    refetch,
  } = useBans(server.id, true);
  const unban = useUnbanMember(server.id);

  return (
    <Section
      id="sperren"
      title="Gesperrte Konten"
      hint="Gesperrte User können auch mit einem Einladungslink nicht wieder beitreten."
    >
      {isPending ? (
        <p className="settings-hint">Lade Sperren…</p>
      ) : error ? (
        <LoadError
          message="Die Sperren konnten gerade nicht geladen werden."
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      ) : !bans || bans.length === 0 ? (
        <p className="settings-hint">Niemand ist gesperrt.</p>
      ) : (
        <ul className="server-settings-list">
          {bans.map((ban) => (
            <li key={ban.user_id} className="server-settings-row">
              <PresenceAvatar name={ban.name} url={ban.avatar_url} status="x" />
              <span className="server-settings-row__name">{ban.name}</span>
              <Button
                size="sm"
                onClick={async () => {
                  if (
                    await confirmAction({
                      title: `${ban.name} wieder zulassen?`,
                      message: "Mit einem Einladungslink geht es dann wieder.",
                      confirmLabel: "Entsperren",
                      tone: "neutral",
                    })
                  )
                    unban.mutate(ban.user_id);
                }}
              >
                Entsperren
              </Button>
            </li>
          ))}
        </ul>
      )}
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
    forget(user.id, server.id);
    void navigate({ to: "/", replace: true });
  };

  return (
    <Section
      id="gefahrenzone"
      hint={
        owner
          ? "Server löschen entfernt Kanäle, Kategorien, Einladungen und Mitgliedschaften. Es gibt kein Zurück."
          : "Du kannst später mit einem neuen Einladungslink wieder beitreten."
      }
    >
      <div className="settings-actions">
        <Button
          variant="danger-outline"
          onClick={async () => {
            if (owner) {
              if (
                !(await confirmAction({
                  title: `Server „${server.name}“ endgültig löschen?`,
                  message:
                    "Kanäle, Nachrichten, Einladungen und Mitgliedschaften sind danach weg.",
                  confirmLabel: "Server löschen",
                }))
              )
                return;
              remove.mutate(server.id);
            } else {
              if (
                !(await confirmAction({
                  title: `Server „${server.name}“ verlassen?`,
                  confirmLabel: "Verlassen",
                }))
              )
                return;
              leave.mutate(server.id);
            }
            // Optimistic: the server is already gone from the rail.
            go();
          }}
        >
          {owner ? "Server löschen" : "Server verlassen"}
        </Button>
      </div>
    </Section>
  );
}
