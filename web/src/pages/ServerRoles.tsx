// Server settings, v0.10 roles (#516): the "Rollen" editor (list and
// panel) and the role chips and picker on a member row. Every change saves
// at once, optimistic like the permission switches before it.

import { useQueryClient } from "@tanstack/react-query";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import { useUserId } from "../auth/scope.ts";
import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { roles as rolesCatalog } from "../i18n/catalogs/roles.ts";
import { serverSettings } from "../i18n/catalogs/serverSettings.ts";
import { Field } from "../components/Field.tsx";
import { LockIcon, PlusIcon } from "../components/Icons.tsx";
import { confirmAction } from "../components/confirm.ts";
import { Button } from "../components/ui/Button.tsx";
import { Checkbox } from "../components/ui/Checkbox.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { Popover } from "../components/ui/Popover.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { PERMISSIONS } from "../servers/permissions.ts";
import {
  isPendingId,
  readServer,
  useCreateRole,
  useDeleteRole,
  useSetMemberRoles,
  useUpdateRole,
  useUpdateServer,
} from "../servers/queries.ts";
import { ROLE_NAME_MAX } from "../servers/rules.ts";
import {
  BASE_ROLE,
  OWNER_ROLE,
  ROLE_COLORS,
  defaultRoleSelection,
  inheritedFromBase,
  newRoleInput,
  roleColorVar,
  roleCounts,
  roleRename,
  rolesInOrder,
  rolesOf,
  toggleMemberRole,
  togglePermission,
} from "../servers/roles.ts";
import type {
  Member,
  Permission,
  RoleColor,
  ServerDetail,
  ServerRole,
} from "../servers/types.ts";
import "./serverRoles.css";

type Status = ReturnType<typeof useInlineStatus>;

/** A role's dot. The owner's is the accent, the base role's muted. */
export function RoleDot({ color }: { color: RoleColor | "owner" | "base" }) {
  const background =
    color === "owner"
      ? "var(--lr-accent)"
      : color === "base"
        ? "var(--lr-muted)"
        : roleColorVar(color);
  return (
    <span
      className="role-dot"
      style={{ background } as CSSProperties}
      aria-hidden="true"
    />
  );
}

/**
 * The list of roles (Inhaber, the custom roles, Alle Mitglieder) and the
 * panel of the chosen one. `initial` picks the first panel; by default the
 * first custom role, else Alle Mitglieder.
 */
export function RolesEditor({
  server,
  initial,
}: {
  server: ServerDetail;
  initial?: string;
}) {
  const t = useT(serverSettings);
  const tr = useT(rolesCatalog);
  const create = useCreateRole(server.id);
  const status = useInlineStatus();
  const [chosen, setChosen] = useState(
    () => initial ?? defaultRoleSelection(server),
  );
  // The new role's name field takes focus once its panel shows.
  const [focusName, setFocusName] = useState<string | null>(null);
  const custom = rolesInOrder(server);
  const counts = roleCounts(server);
  // A deleted role (here or elsewhere) falls back to the default panel.
  const selected =
    chosen === OWNER_ROLE ||
    chosen === BASE_ROLE ||
    custom.some((role) => role.id === chosen)
      ? chosen
      : defaultRoleSelection(server);

  const addRole = async () => {
    status.clear();
    try {
      // The role is editable once it has its real id.
      const role = await create.mutateAsync(
        newRoleInput(server, t("roles.new")),
      );
      setChosen(role.id);
      setFocusName(role.id);
    } catch {
      // The hook shows the toast and drops the row.
    }
  };

  const row = (
    id: string,
    dot: RoleColor | "owner" | "base",
    name: string,
    count: number,
    locked = false,
  ) => (
    <li key={id}>
      <button
        type="button"
        className="roles-editor__row"
        aria-current={id === selected ? "true" : undefined}
        disabled={isPendingId(id)}
        onClick={() => {
          status.clear();
          setChosen(id);
        }}
      >
        <RoleDot color={dot} />
        <span className="roles-editor__name">{name}</span>
        {locked ? <LockIcon size={14} className="roles-editor__lock" /> : null}
        <span className="roles-editor__count">{count}</span>
      </button>
    </li>
  );

  const role = custom.find((r) => r.id === selected);
  return (
    <div className="roles-editor">
      <div className="roles-editor__side">
        <ul className="roles-editor__list">
          {row(OWNER_ROLE, "owner", tr("owner"), counts.owner, true)}
          {custom.map((r) =>
            row(r.id, r.color, r.name, counts.roles[r.id] ?? 0),
          )}
          {row(BASE_ROLE, "base", tr("base"), counts.base)}
        </ul>
        <Button
          className="roles-editor__new"
          icon={<PlusIcon size={16} />}
          disabled={create.isPending}
          onClick={() => void addRole()}
        >
          {t("roles.new")}
        </Button>
      </div>
      <div className="roles-editor__panel">
        {role ? (
          <CustomRolePanel
            key={role.id}
            server={server}
            role={role}
            status={status}
            focusName={focusName === role.id}
          />
        ) : selected === OWNER_ROLE ? (
          <OwnerPanel />
        ) : (
          <BasePanel server={server} status={status} />
        )}
        <InlineStatus status={status.status} />
      </div>
    </div>
  );
}

/** The nine switches, in canonical order. */
function PermissionSwitches({
  checked,
  inherited,
  disabled,
  onToggle,
}: {
  checked: (permission: Permission) => boolean;
  /** The flags shown on and fixed, with the note "über „Alle Mitglieder“". */
  inherited?: (permission: Permission) => boolean;
  disabled?: boolean;
  onToggle?: (permission: Permission) => void;
}) {
  const t = useT(serverSettings);
  const tr = useT(rolesCatalog);
  const titleId = useId();
  return (
    <div className="roles-editor__group" role="group" aria-labelledby={titleId}>
      <h4 id={titleId} className="roles-editor__label">
        {t("roles.permissionsLabel")}
      </h4>
      <div className="roles-editor__permissions">
        {PERMISSIONS.map((permission) => {
          const fixed = inherited?.(permission.id) ?? false;
          return (
            <label
              key={permission.id}
              className="ui-choice-row ui-choice-row--between roles-permission"
            >
              <span className="ui-choice-text">
                {t(`permission.${permission.key}.label`)}
                <small>{t(`permission.${permission.key}.hint`)}</small>
              </span>
              {fixed ? (
                <span className="roles-permission__note">
                  {t("roles.inherited", { role: tr("base") })}
                </span>
              ) : null}
              <Switch
                checked={fixed || checked(permission.id)}
                disabled={disabled || fixed}
                onChange={() => onToggle?.(permission.id)}
              />
            </label>
          );
        })}
      </div>
    </div>
  );
}

function PanelHeader({ title, action }: { title: string; action?: ReactNode }) {
  return (
    <div className="roles-editor__header">
      <h3>{title}</h3>
      {action}
    </div>
  );
}

function OwnerPanel() {
  const t = useT(serverSettings);
  const tr = useT(rolesCatalog);
  return (
    <>
      <PanelHeader title={tr("owner")} />
      <p className="roles-editor__text">{t("roles.ownerText")}</p>
      <PermissionSwitches checked={() => true} disabled />
    </>
  );
}

/** "Alle Mitglieder" is `member_permissions`: fixed name, no colour. */
function BasePanel({
  server,
  status,
}: {
  server: ServerDetail;
  status: Status;
}) {
  const tc = useT(common);
  const tr = useT(rolesCatalog);
  const client = useQueryClient();
  const userId = useUserId();
  const update = useUpdateServer(server.id);
  const current = new Set<Permission>(server.member_permissions);

  const toggle = (permission: Permission) => {
    // Start from the cache, not from this render's props: a second click
    // before the first PATCH returns must build on the first click's
    // optimistic mask, or the later request would restore the flag.
    const latest = (userId && readServer(client, userId, server.id)) || server;
    update.mutate(
      {
        member_permissions: togglePermission(
          latest.member_permissions,
          permission,
        ),
      },
      {
        onSuccess: () => status.show({ tone: "success", message: tc("saved") }),
        onError: () => status.show({ tone: "error", message: tc("notSaved") }),
      },
    );
  };

  return (
    <>
      <PanelHeader title={tr("base")} />
      <PermissionSwitches
        checked={(permission) => current.has(permission)}
        onToggle={toggle}
      />
    </>
  );
}

function CustomRolePanel({
  server,
  role,
  status,
  focusName,
}: {
  server: ServerDetail;
  role: ServerRole;
  status: Status;
  focusName: boolean;
}) {
  const t = useT(serverSettings);
  const tc = useT(common);
  const client = useQueryClient();
  const userId = useUserId();
  const update = useUpdateRole(server.id);
  const remove = useDeleteRole(server.id);
  const [name, setName] = useState(role.name);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const nameField = useRef<HTMLInputElement>(null);
  const colorId = useId();

  useEffect(() => {
    if (!focusName) return;
    nameField.current?.focus();
    nameField.current?.select();
  }, [focusName]);

  /** The role as the cache holds it now, for clicks in quick succession. */
  const latest = () =>
    ((userId && readServer(client, userId, server.id)) || server).roles.find(
      (r) => r.id === role.id,
    ) ?? role;

  const saved = {
    onSuccess: () => status.show({ tone: "success", message: tc("saved") }),
    onError: () => status.show({ tone: "error", message: tc("notSaved") }),
  } as const;

  const rename = () => {
    const change = roleRename(server, latest(), name);
    if (!change) return;
    if ("code" in change) {
      setFields({ name: fieldMessage("name", change.code) });
      return;
    }
    status.clear();
    update.mutate(
      { id: role.id, patch: { name: change.name } },
      {
        onSuccess: saved.onSuccess,
        onError: (error) => {
          fromError(error);
          saved.onError();
        },
      },
    );
  };

  const recolor = (color: RoleColor) => {
    if (latest().color === color) return;
    status.clear();
    update.mutate({ id: role.id, patch: { color } }, saved);
  };

  const toggle = (permission: Permission) => {
    status.clear();
    update.mutate(
      {
        id: role.id,
        patch: {
          permissions: togglePermission(latest().permissions, permission),
        },
      },
      saved,
    );
  };

  const deleteRole = async () => {
    if (
      await confirmAction({
        title: t("roles.deleteTitle", { name: role.name }),
        message: t("roles.deleteMessage"),
        confirmLabel: tc("delete"),
      })
    )
      remove.mutate(role.id);
  };

  const owned = new Set<Permission>(role.permissions);
  return (
    <>
      <PanelHeader
        title={role.name}
        action={
          <Button
            size="sm"
            variant="danger-outline"
            onClick={() => void deleteRole()}
          >
            {t("roles.delete")}
          </Button>
        }
      />
      <form
        className="roles-editor__name-form"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          rename();
        }}
      >
        <Field
          ref={nameField}
          id={`role-name-${role.id}`}
          label={t("roles.nameLabel")}
          type="text"
          maxLength={ROLE_NAME_MAX}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            clearField("name");
          }}
          onBlur={rename}
          error={errors.fields.name}
        />
      </form>
      <div
        className="roles-editor__group"
        role="radiogroup"
        aria-labelledby={colorId}
      >
        <h4 id={colorId} className="roles-editor__label">
          {t("roles.colorLabel")}
        </h4>
        <div className="roles-editor__swatches">
          {ROLE_COLORS.map((color) => (
            <input
              key={color}
              type="radio"
              name={`role-color-${role.id}`}
              className="role-swatch"
              style={{ background: roleColorVar(color) } as CSSProperties}
              aria-label={t(`roles.colorName.${color}`)}
              title={t(`roles.colorName.${color}`)}
              checked={role.color === color}
              onChange={() => recolor(color)}
            />
          ))}
        </div>
      </div>
      <PermissionSwitches
        checked={(permission) => owned.has(permission)}
        inherited={(permission) => inheritedFromBase(server, permission)}
        onToggle={toggle}
      />
    </>
  );
}

/** A member's custom roles as chips (dot and name), for every viewer. */
export function MemberRoleChips({
  server,
  member,
}: {
  server: ServerDetail;
  member: Member;
}) {
  const held = rolesOf(member, server);
  if (held.length === 0) return null;
  return (
    <ul className="role-chips">
      {held.map((role) => (
        <li key={role.id} className="role-chip">
          <RoleDot color={role.color} />
          {role.name}
        </li>
      ))}
    </ul>
  );
}

/**
 * The dashed "+" on a member row (with `manage_server`): a checklist of the
 * custom roles that saves each click at once and stays open, and a link to
 * the Rollen section.
 */
export function MemberRolePicker({
  server,
  member,
  onManage,
}: {
  server: ServerDetail;
  member: Member;
  onManage: () => void;
}) {
  const t = useT(serverSettings);
  const client = useQueryClient();
  const userId = useUserId();
  const assign = useSetMemberRoles(server.id);
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const label = t("members.rolesFor", { name: member.name });
  const held = new Set(member.role_ids);
  const roles = rolesInOrder(server);

  const toggle = (id: string) => {
    // From the cache, so a second click builds on the first.
    const latest = (userId && readServer(client, userId, server.id)) || server;
    const current =
      latest.members.find((m) => m.user_id === member.user_id) ?? member;
    assign.mutate({
      userId: member.user_id,
      roleIds: toggleMemberRole(current.role_ids, id),
    });
  };

  return (
    <>
      <IconButton
        ref={button}
        label={label}
        icon={<PlusIcon size={14} />}
        size="xs"
        className="role-picker-button"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={button}
        label={label}
        placement="bottom-end"
        className="role-picker"
      >
        <p className="role-picker__title">{label}</p>
        {roles.length === 0 ? (
          <p className="role-picker__empty">{t("members.noRoles")}</p>
        ) : (
          <div className="role-picker__list">
            {roles.map((role) => (
              <Checkbox
                key={role.id}
                className="role-picker__row"
                label={
                  <span className="role-picker__role">
                    <RoleDot color={role.color} />
                    {role.name}
                  </span>
                }
                checked={held.has(role.id)}
                disabled={isPendingId(role.id)}
                onChange={() => toggle(role.id)}
              />
            ))}
          </div>
        )}
        <hr className="role-picker__divider" />
        <button
          type="button"
          className="role-picker__manage"
          onClick={() => {
            setOpen(false);
            onManage();
          }}
        >
          {t("members.manageRoles")}
        </button>
      </Popover>
    </>
  );
}
