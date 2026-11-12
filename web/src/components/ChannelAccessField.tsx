// "Zugriff" in "Kanal erstellen" and "Kanal bearbeiten" (v0.10 roles,
// #516): a field drawn like the "Kategorie" select that opens a checklist
// of "Alle Mitglieder" and the server's roles.

import { useId, useRef, useState } from "react";

import { useT } from "../i18n/catalog.ts";
import { roles as rolesCatalog } from "../i18n/catalogs/roles.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import { isPendingId } from "../servers/queries.ts";
import {
  BASE_ROLE,
  accessSummary,
  rolesInOrder,
  toggleAccess,
  type AccessSelection,
} from "../servers/roles.ts";
import type { ServerDetail } from "../servers/types.ts";
import { ChevronIcon } from "./Icons.tsx";
import { RoleDot } from "./RoleDot.tsx";
import { Checkbox } from "./ui/Checkbox.tsx";
import { FormField } from "./ui/FormField.tsx";
import { Popover } from "./ui/Popover.tsx";
import "./channelAccess.css";

export function ChannelAccessField({
  server,
  value,
  onChange,
}: {
  server: Pick<ServerDetail, "roles">;
  value: AccessSelection;
  onChange: (next: AccessSelection) => void;
}) {
  const t = useT(servers);
  const tr = useT(rolesCatalog);
  const id = useId();
  const valueId = `${id}-value`;
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const roles = rolesInOrder(server);
  const held = new Set(value ?? []);
  const summary = accessSummary(
    { limited: value !== null, role_ids: value ?? [] },
    server,
  );

  return (
    <FormField
      id={id}
      label={t("channel.accessLabel")}
      hint={t("channel.accessHint")}
    >
      <span className="ui-select">
        <button
          ref={button}
          id={id}
          type="button"
          className="ui-field channel-access-trigger"
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-describedby={`${valueId} ${id}-hint`}
          onClick={() => setOpen((was) => !was)}
        >
          <span id={valueId} className="channel-access-value">
            {summary}
          </span>
        </button>
        <ChevronIcon size={14} />
      </span>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={button}
        label={t("channel.accessLabel")}
        className="channel-access"
        matchAnchorWidth
      >
        <div className="channel-access__list">
          <Checkbox
            className="channel-access__row"
            label={
              <span className="channel-access__role">
                <RoleDot color="base" />
                {tr("base")}
              </span>
            }
            checked={value === null}
            onChange={() => onChange(toggleAccess(value, BASE_ROLE))}
          />
          {roles.map((role) => (
            <Checkbox
              key={role.id}
              className="channel-access__row"
              label={
                <span className="channel-access__role">
                  <RoleDot color={role.color} />
                  {role.name}
                </span>
              }
              checked={held.has(role.id)}
              disabled={isPendingId(role.id)}
              onChange={() => onChange(toggleAccess(value, role.id))}
            />
          ))}
        </div>
        {roles.length === 0 ? (
          <p className="channel-access__empty">{t("channel.accessNoRoles")}</p>
        ) : null}
      </Popover>
    </FormField>
  );
}
