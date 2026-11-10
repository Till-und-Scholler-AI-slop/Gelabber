// `/s/$serverId/settings`: name, roles, invites, members, bans,
// leave/delete. Open to every member; `serverSettingsSections` says who sees
// which section, deletion needs the owner.

import { useNavigate, useParams } from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type FormEvent,
  type ReactNode,
  type RefObject,
} from "react";

import { useSession } from "../auth/session.ts";
import { ApiError } from "../api/client.ts";
import {
  AvatarImageError,
  avatarImageMessage,
  prepareAvatar,
} from "../auth/avatarImage.ts";
import { errorMessage, fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { serverSettings } from "../i18n/catalogs/serverSettings.ts";
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
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { inviteUrl } from "../servers/api.ts";
import { copyText } from "../servers/clipboard.ts";
import { inviteExpiry, inviteUses } from "../servers/inviteText.ts";
import { useLastChannel } from "../servers/lastChannel.ts";
import { can } from "../servers/permissions.ts";
import {
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
import type { Invite, ServerDetail } from "../servers/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { sectionInView, type SectionJump } from "./sectionSpy.ts";
import {
  MemberRoleChips,
  MemberRolePicker,
  RolesEditor,
} from "./ServerRoles.tsx";
import {
  serverSettingsSections,
  type SectionId,
} from "./serverSettingsSections.ts";
import "./settings.css";

export function ServerSettingsPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId/settings" });
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  const t = useT(serverSettings);
  if (error && !server)
    return (
      <div className="p-6">
        <LoadError
          message={t("loadFailed")}
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      </div>
    );
  if (!server) return null;
  return <Settings key={server.id} server={server} />;
}

/** Each section's label; the ids stay German, they are URL fragments. */
const SECTION_LABEL = {
  allgemein: "section.general",
  rollen: "section.roles",
  einladungen: "section.invites",
  mitglieder: "section.members",
  sperren: "section.bans",
  gefahrenzone: "section.danger",
} as const satisfies Record<SectionId, string>;

const sectionId = (id: SectionId) => `server-settings-${id}`;

/**
 * The section in view, for the anchor row. A jump marks its section even
 * where the page cannot scroll that heading up to the row (the short last
 * ones), until the reader scrolls on.
 */
function useCurrentSection(
  sections: readonly SectionId[],
  scroller: RefObject<HTMLDivElement | null>,
  nav: RefObject<HTMLElement | null>,
) {
  const [current, setCurrent] = useState(sections[0]);
  const jumped = useRef<SectionJump<SectionId> | null>(null);
  const key = sections.join(" ");

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const ids = key.split(" ") as SectionId[];
    const update = () => {
      const line = nav.current?.getBoundingClientRect().bottom ?? 0;
      const headings = ids.flatMap((id) => {
        const heading = document.getElementById(sectionId(id));
        return heading
          ? [{ id, top: heading.getBoundingClientRect().top - line }]
          : [];
      });
      const view = sectionInView({
        headings,
        scrollTop: element.scrollTop,
        clientHeight: element.clientHeight,
        scrollHeight: element.scrollHeight,
        jump: jumped.current,
      });
      jumped.current = view.jump;
      setCurrent((before) => view.current ?? before);
    };
    update();
    element.addEventListener("scroll", update, { passive: true });
    // Invites and bans load later and move the sections below them.
    const resized =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    resized?.observe(element);
    if (element.lastElementChild) resized?.observe(element.lastElementChild);
    return () => {
      element.removeEventListener("scroll", update);
      resized?.disconnect();
    };
  }, [key, scroller, nav]);

  // On a narrow window the row scrolls sideways: keep the marked link in it.
  useEffect(() => {
    const row = nav.current;
    const link = row?.querySelector('[aria-current="location"]');
    if (!row || !link || row.scrollWidth <= row.clientWidth) return;
    const bounds = row.getBoundingClientRect();
    const box = link.getBoundingClientRect();
    if (box.left < bounds.left)
      row.scrollBy({ left: box.left - bounds.left - 32 });
    else if (box.right > bounds.right)
      row.scrollBy({ left: box.right - bounds.right + 32 });
  }, [current, nav]);

  const jump = (id: SectionId) => {
    const element = scroller.current;
    const heading = document.getElementById(sectionId(id));
    if (!element || !heading) return;
    // Only this scroller moves: scrollIntoView would also shift the shell
    // around it (it scrolls clipped ancestors too).
    const margin = parseFloat(getComputedStyle(heading).scrollMarginTop) || 0;
    element.scrollBy({
      top:
        heading.getBoundingClientRect().top -
        element.getBoundingClientRect().top -
        margin,
    });
    heading.focus({ preventScroll: true });
    jumped.current = { id, scrollTop: element.scrollTop };
    setCurrent(id);
  };
  return [current, jump] as const;
}

function Settings({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  // The directory switch arrives with discovery.
  const sections = serverSettingsSections(server, false);
  const shows = (id: SectionId) => sections.includes(id);
  const scroller = useRef<HTMLDivElement>(null);
  const nav = useRef<HTMLElement>(null);
  const [current, jump] = useCurrentSection(sections, scroller, nav);
  return (
    <div className="settings-screen">
      <header className="lr-channel-header settings-header">
        <h1>
          {t("title")}
          <span className="settings-header-context"> · {server.name}</span>
        </h1>
      </header>
      <div ref={scroller} className="settings-scroll">
        <nav
          ref={nav}
          className="server-settings-nav"
          aria-label={t("navLabel")}
        >
          {sections.map((id) => (
            <a
              key={id}
              href={`#${sectionId(id)}`}
              aria-current={id === current ? "location" : undefined}
              onClick={(event) => {
                // Scroll inside the page; the address stays the server's.
                event.preventDefault();
                jump(id);
              }}
            >
              {t(SECTION_LABEL[id])}
            </a>
          ))}
        </nav>
        <div className="settings-area server-settings">
          {shows("allgemein") ? <NameSection server={server} /> : null}
          {shows("rollen") ? <RolesSection server={server} /> : null}
          {shows("einladungen") ? <InvitesSection server={server} /> : null}
          <MembersSection
            server={server}
            onManageRoles={() => jump("rollen")}
          />
          {shows("sperren") ? <BansSection server={server} /> : null}
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
  className,
  children,
}: {
  id: SectionId;
  title?: string;
  hint?: string;
  className?: string;
  children: ReactNode;
}) {
  const t = useT(serverSettings);
  return (
    <section
      className={["server-settings-section", className]
        .filter(Boolean)
        .join(" ")}
      aria-labelledby={sectionId(id)}
    >
      <header className="server-settings-section__heading">
        <h2 id={sectionId(id)} tabIndex={-1}>
          {title ?? t(SECTION_LABEL[id])}
        </h2>
        {hint ? <p>{hint}</p> : null}
      </header>
      {children}
    </section>
  );
}

function NameSection({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const tc = useT(common);
  const [name, setName] = useState(server.name);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const update = useUpdateServer(server.id);
  const status = useInlineStatus();
  const dirty = name.trim() !== server.name;
  // As on Profil: why Speichern waits, next to it.
  const unchanged = !dirty && !update.isPending && !status.status;
  const unchangedId = useId();

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
        onSuccess: () => status.show({ tone: "success", message: tc("saved") }),
        onError: fromError,
      },
    );
  };

  return (
    <Section id="allgemein" hint={t("general.hint")}>
      <IconRow server={server} />
      <form onSubmit={onSubmit} noValidate className="settings-form">
        <Field
          id="server-name"
          label={t("general.nameLabel")}
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
            pendingLabel={t("general.saving")}
            disabled={!dirty}
            aria-describedby={unchanged ? unchangedId : undefined}
          >
            {tc("save")}
          </SubmitButton>
          {unchanged ? (
            <span id={unchangedId} className="settings-hint">
              {t("general.unchanged")}
            </span>
          ) : null}
          <InlineStatus status={status.status} />
        </div>
      </form>
    </Section>
  );
}

/** Upload and removal apply at once, like the profile avatar. */
function IconRow({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const tc = useT(common);
  const picker = useRef<HTMLInputElement>(null);
  const icon = useServerIcon(server.id);
  const status = useInlineStatus();
  const [removing, setRemoving] = useState(false);
  const save = (image: Blob | null) => {
    status.clear();
    setRemoving(image === null);
    icon.mutate(image, {
      onSuccess: () => status.show({ tone: "success", message: tc("saved") }),
      onError: (error) =>
        status.show({
          tone: "error",
          message:
            error instanceof AvatarImageError
              ? avatarImageMessage(error)
              : error instanceof ApiError
                ? errorMessage(error.code)
                : tc("notSaved"),
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
        <div className="settings-actions">
          <Button
            size="sm"
            onClick={() => picker.current?.click()}
            disabled={icon.isPending}
            pending={icon.isPending && !removing}
            pendingLabel={t("icon.uploading")}
          >
            {t("icon.upload")}
          </Button>
          {server.icon_url ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => save(null)}
              disabled={icon.isPending}
              pending={icon.isPending && removing}
              pendingLabel={t("icon.removing")}
            >
              {tc("remove")}
            </Button>
          ) : null}
          <InlineStatus status={status.status} />
        </div>
        <input
          ref={picker}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          aria-label={t("icon.pickLabel")}
          onChange={(event) => void onPick(event)}
        />
      </div>
    </div>
  );
}

/** Roles: the list and the chosen role's panel; each change saves at once. */
function RolesSection({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  return (
    <Section
      id="rollen"
      hint={t("roles.hint")}
      className="server-settings-section--roles"
    >
      <RolesEditor server={server} />
    </Section>
  );
}

function InvitesSection({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
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
      message: t("invites.copyFailed", { code: invite.code }),
    });
    return false;
  };

  return (
    <Section id="einladungen" hint={t("invites.hint")}>
      <div className="settings-actions">
        <Button onClick={() => setInviting(true)}>{t("invites.create")}</Button>
        <InlineStatus status={status.status} />
      </div>
      <InviteDialog
        server={server}
        open={inviting}
        onClose={() => setInviting(false)}
      />
      {isPending ? (
        <p className="settings-hint">{t("invites.loading")}</p>
      ) : error ? (
        <LoadError
          message={t("invites.loadFailed")}
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      ) : !invites || invites.length === 0 ? (
        <p className="settings-hint">{t("invites.empty")}</p>
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
                    title: t("invites.revokeTitle"),
                    message: t("invites.revokeMessage"),
                    confirmLabel: t("invites.revoke"),
                  })
                )
                  revoke.mutate(invite.code, {
                    onSuccess: () =>
                      status.show({
                        tone: "success",
                        message: t("invites.revoked"),
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
  const t = useT(serverSettings);
  const tc = useT(common);
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
          {tc("copy")}
        </CopyButton>
        <Button size="sm" variant="danger-outline" onClick={onRevoke}>
          {t("invites.revoke")}
        </Button>
      </div>
    </li>
  );
}

function MembersSection({
  server,
  onManageRoles,
}: {
  server: ServerDetail;
  onManageRoles: () => void;
}) {
  const t = useT(serverSettings);
  const byServer = usePresenceStore((s) => s.byServer);
  const manage = can(server, "manage_server");
  const moderate = can(server, "moderate_members");
  const me = useSession((s) => s.user?.id);
  return (
    <Section
      id="mitglieder"
      title={t("members.title", { count: server.members.length })}
      hint={manage ? t("members.hint") : undefined}
    >
      <ul className="server-settings-list">
        {server.members.map((member) => (
          <li key={member.user_id} className="server-settings-row">
            <PresenceAvatar
              name={member.name}
              url={member.avatar_url}
              status={presenceOf(byServer, server.id, member.user_id)}
            />
            <span className="server-settings-row__name">{member.name}</span>
            {member.role === "owner" ? (
              <Badge tone="accent">{t("members.owner")}</Badge>
            ) : (
              <>
                <MemberRoleChips server={server} member={member} />
                {manage ? (
                  <MemberRolePicker
                    server={server}
                    member={member}
                    onManage={onManageRoles}
                  />
                ) : null}
              </>
            )}
            {moderate && (member.role === "owner" || member.user_id === me) ? (
              // No "…" here: keep its column, so the badge and chips line up.
              <span className="server-settings-row__menu-slot" aria-hidden />
            ) : (
              <MemberActionsMenu server={server} member={member} />
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}

function BansSection({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const {
    data: bans,
    isPending,
    error,
    isFetching,
    refetch,
  } = useBans(server.id, true);
  const unban = useUnbanMember(server.id);

  return (
    <Section id="sperren" title={t("bans.title")} hint={t("bans.hint")}>
      {isPending ? (
        <p className="settings-hint">{t("bans.loading")}</p>
      ) : error ? (
        <LoadError
          message={t("bans.loadFailed")}
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      ) : !bans || bans.length === 0 ? (
        <p className="settings-hint">{t("bans.empty")}</p>
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
                      title: t("bans.unbanTitle", { name: ban.name }),
                      message: t("bans.unbanMessage"),
                      confirmLabel: t("bans.unban"),
                      tone: "neutral",
                    })
                  )
                    unban.mutate(ban.user_id);
                }}
              >
                {t("bans.unban")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function DangerSection({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
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
      hint={owner ? t("danger.ownerHint") : t("danger.memberHint")}
    >
      <div className="settings-actions">
        <Button
          variant="danger-outline"
          onClick={async () => {
            if (owner) {
              if (
                !(await confirmAction({
                  title: t("danger.deleteTitle", { name: server.name }),
                  message: t("danger.deleteMessage"),
                  confirmLabel: t("danger.delete"),
                }))
              )
                return;
              remove.mutate(server.id);
            } else {
              if (
                !(await confirmAction({
                  title: t("danger.leaveTitle", { name: server.name }),
                  confirmLabel: t("danger.leaveConfirm"),
                }))
              )
                return;
              leave.mutate(server.id);
            }
            // Optimistic: the server is already gone from the rail.
            go();
          }}
        >
          {owner ? t("danger.delete") : t("danger.leave")}
        </Button>
      </div>
    </Section>
  );
}
