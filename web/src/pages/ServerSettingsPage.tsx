// `/s/$serverId/settings`: name, roles, invites, the directory, members,
// bans, leave, transfer and delete. Open to every member; `serverSettingsSections`
// says who sees which section and row; transfer and deletion need the owner.

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
import { ActionRow } from "../components/ActionRow.tsx";
import { Field } from "../components/Field.tsx";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { tileInitials, tileTint } from "../components/initials.ts";
import { confirmAction } from "../components/confirm.ts";
import { LoadError } from "../components/LoadError.tsx";
import { FormError } from "../components/FormError.tsx";
import { InviteDialog } from "../components/InviteDialog.tsx";
import { GhostButton, Modal } from "../components/Modal.tsx";
import { MemberActionsMenu } from "../components/MemberActions.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { Button } from "../components/ui/Button.tsx";
import { FormField } from "../components/ui/FormField.tsx";
import { Select } from "../components/ui/Select.tsx";
import { CopyButton } from "../components/ui/CopyButton.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import {
  useDirectoryStatus,
  useJoinRequestCount,
} from "../discover/queries.ts";
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
  useTransferServer,
  useUnbanMember,
  useUpdateServer,
} from "../servers/queries.ts";
import { validateName } from "../servers/rules.ts";
import type { DirectoryLanguage } from "../discover/types.ts";
import type { Invite, ServerDetail } from "../servers/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { sectionInView, type SectionJump } from "./sectionSpy.ts";
import { JoinRequestQueue, ListingSettingsPanel } from "./ServerDirectory.tsx";
import {
  MemberRoleChips,
  MemberRolePicker,
  RolesEditor,
} from "./ServerRoles.tsx";
import {
  dangerRows,
  serverSettingsSections,
  transferCandidates,
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
  verzeichnis: "section.directory",
} as const satisfies Record<SectionId, string>;

const sectionId = (id: SectionId) => `server-settings-${id}`;

/** The section the address names (`#server-settings-…`), if any. */
function openedSection(sections: readonly SectionId[]): SectionId | null {
  if (typeof window === "undefined") return null;
  const hash = window.location.hash.slice(1);
  return sections.find((id) => sectionId(id) === hash) ?? null;
}

/**
 * Brings a section's heading up under the anchor row. Only this scroller
 * moves: scrollIntoView would also shift the shell around it (it scrolls
 * clipped ancestors too).
 */
function scrollToSection(
  element: HTMLElement,
  id: SectionId,
): HTMLElement | null {
  const heading = document.getElementById(sectionId(id));
  if (!heading) return null;
  const margin = parseFloat(getComputedStyle(heading).scrollMarginTop) || 0;
  element.scrollBy({
    top:
      heading.getBoundingClientRect().top -
      element.getBoundingClientRect().top -
      margin,
  });
  return heading;
}

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
  // Opened at a section (`#server-settings-gefahrenzone` from Konto
  // löschen): go there and mark it, again while content loading above moves
  // it, until the reader scrolls or clicks by themselves.
  const opened = useRef(openedSection(sections));
  const key = sections.join(" ");

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const ids = key.split(" ") as SectionId[];
    const follow = () => {
      const id = opened.current;
      if (!id || !ids.includes(id) || !scrollToSection(element, id)) return;
      jumped.current = { id, scrollTop: element.scrollTop };
      setCurrent(id);
    };
    const release = () => {
      opened.current = null;
    };
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
    const relayout = () => {
      follow();
      update();
    };
    relayout();
    element.addEventListener("scroll", update, { passive: true });
    const own = ["wheel", "touchstart", "keydown", "pointerdown"] as const;
    for (const type of own)
      element.addEventListener(type, release, { passive: true });
    // Invites and bans load later and move the sections below them.
    const resized =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(relayout);
    resized?.observe(element);
    if (element.lastElementChild) resized?.observe(element.lastElementChild);
    return () => {
      element.removeEventListener("scroll", update);
      for (const type of own) element.removeEventListener(type, release);
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
    const heading = element && scrollToSection(element, id);
    if (!element || !heading) return;
    heading.focus({ preventScroll: true });
    jumped.current = { id, scrollTop: element.scrollTop };
    setCurrent(id);
  };
  return [current, jump] as const;
}

function Settings({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const { data: directory } = useDirectoryStatus();
  const sections = serverSettingsSections(server, directory?.enabled ?? false);
  const shows = (id: SectionId) => sections.includes(id);
  const scroller = useRef<HTMLDivElement>(null);
  const nav = useRef<HTMLElement>(null);
  const [current, jump] = useCurrentSection(sections, scroller, nav);
  // Only read here: the queue in the section loads it, `jq` keeps it current.
  const { data: requestCount } = useJoinRequestCount(server.id, false);
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
              {id === "verzeichnis" && requestCount ? (
                <Badge tone="count">
                  <span aria-hidden="true">{requestCount}</span>
                  <span className="sr-only">
                    {t("joinRequests.count", { count: requestCount })}
                  </span>
                </Badge>
              ) : null}
            </a>
          ))}
        </nav>
        <div className="settings-area server-settings">
          {shows("allgemein") ? <NameSection server={server} /> : null}
          {shows("rollen") ? <RolesSection server={server} /> : null}
          {shows("einladungen") ? <InvitesSection server={server} /> : null}
          {shows("verzeichnis") ? (
            <DirectorySection
              server={server}
              languages={directory?.languages ?? []}
            />
          ) : null}
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

/** The owner's listing, then the join requests for everyone who answers. */
function DirectorySection({
  server,
  languages,
}: {
  server: ServerDetail;
  languages: readonly DirectoryLanguage[];
}) {
  const t = useT(serverSettings);
  const owner = server.role === "owner";
  return (
    <Section id="verzeichnis" hint={owner ? t("directory.hint") : undefined}>
      {owner ? (
        <ListingSettingsPanel server={server} languages={languages} />
      ) : null}
      <JoinRequestQueue server={server} />
    </Section>
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

  const act = async () => {
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
  };

  // The owner's rows each say what they do; the deletion sentence is the
  // hint of Server löschen, not of the section above Server übergeben.
  if (dangerRows(server).includes("transfer"))
    return (
      <Section id="gefahrenzone">
        <ul className="settings-action-list">
          <TransferRow server={server} />
          <ActionRow
            title={t("danger.delete")}
            hint={t("danger.ownerHint")}
            status={null}
            action={(describedBy) => (
              <Button
                size="sm"
                variant="danger-outline"
                aria-describedby={describedBy}
                onClick={() => void act()}
              >
                {t("danger.delete")}
              </Button>
            )}
          />
        </ul>
      </Section>
    );

  return (
    <Section id="gefahrenzone" hint={t("danger.memberHint")}>
      <div className="settings-actions">
        <Button variant="danger-outline" onClick={() => void act()}>
          {t("danger.leave")}
        </Button>
      </div>
    </Section>
  );
}

/** "Server übergeben", above "Server löschen" (#519). */
function TransferRow({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const [open, setOpen] = useState(false);
  const none = transferCandidates(server).length === 0;
  return (
    <ActionRow
      title={t("transfer.title")}
      hint={none ? t("transfer.noMember") : t("transfer.hint")}
      status={null}
      action={(describedBy) => (
        <Button
          size="sm"
          aria-describedby={describedBy}
          disabled={none}
          onClick={() => setOpen(true)}
        >
          {t("transfer.open")}
        </Button>
      )}
    >
      <TransferDialog
        server={server}
        open={open}
        onClose={() => setOpen(false)}
      />
    </ActionRow>
  );
}

/** Pick the new owner; a confirmation names them before anything is sent. */
export function TransferDialog({
  server,
  open,
  onClose,
}: {
  server: ServerDetail;
  open: boolean;
  onClose: () => void;
}) {
  const t = useT(serverSettings);
  const tc = useT(common);
  const form = useRef<HTMLFormElement>(null);
  const candidates = transferCandidates(server);
  const [picked, setPicked] = useState("");
  // The first member until one is picked, or when the picked one has left.
  const chosen =
    candidates.find((member) => member.user_id === picked) ?? candidates[0];
  const { errors, clear, clearField, fromError } = useFormErrors(form);
  const transfer = useTransferServer(server.id);

  const close = () => {
    setPicked("");
    clear();
    onClose();
  };

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!chosen || transfer.isPending) return;
    if (
      !(await confirmAction({
        title: t("transfer.confirmTitle", {
          server: server.name,
          name: chosen.name,
        }),
        message: t("transfer.confirmMessage"),
        confirmLabel: t("transfer.submit"),
      }))
    )
      return;
    transfer.mutate(chosen.user_id, { onSuccess: close, onError: fromError });
  };

  return (
    <Modal open={open} onClose={close} title={t("transfer.title")}>
      <form
        ref={form}
        onSubmit={(event) => void onSubmit(event)}
        noValidate
        className="settings-form"
      >
        <FormField
          id="transfer-member"
          label={t("transfer.member")}
          error={errors.fields.user_id}
        >
          <Select
            id="transfer-member"
            value={chosen?.user_id ?? ""}
            invalid={Boolean(errors.fields.user_id)}
            onChange={(event) => {
              setPicked(event.target.value);
              clearField("user_id");
            }}
          >
            {candidates.map((member) => (
              <option key={member.user_id} value={member.user_id}>
                {member.name}
              </option>
            ))}
          </Select>
        </FormField>
        <FormError message={errors.form} />
        <div className="settings-dialog-actions">
          <GhostButton onClick={close}>{tc("cancel")}</GhostButton>
          <Button
            type="submit"
            variant="danger"
            disabled={!chosen}
            pending={transfer.isPending}
            pendingLabel={t("transfer.pending")}
          >
            {t("transfer.submit")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
