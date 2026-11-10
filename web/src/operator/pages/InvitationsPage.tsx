// "Beta-Einladungen": the registration mode, a form that makes an
// invitation link (optionally for one address, optionally mailed), and
// every invitation with its state.

import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../../api/client.ts";
import { useFormErrors } from "../../auth/useFormErrors.ts";
import { Field } from "../../components/Field.tsx";
import { FormError } from "../../components/FormError.tsx";
import { LoadError } from "../../components/LoadError.tsx";
import { Badge } from "../../components/ui/Badge.tsx";
import { Button } from "../../components/ui/Button.tsx";
import { Checkbox } from "../../components/ui/Checkbox.tsx";
import { ChipGroup } from "../../components/ui/ChipGroup.tsx";
import { CopyButton } from "../../components/ui/CopyButton.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { useT } from "../../i18n/catalog.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { formatDate } from "../../i18n/format.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import * as remote from "../api.ts";
import { formatWhen } from "../format.ts";
import {
  EMPTY_INVITATION_FORM,
  EXPIRY_CHOICES,
  NOTE_MAX,
  canSendMail,
  checkInvitationForm,
  copyInvitationLink,
  invitationFieldMessages,
  invitationInput,
  invitationMenuEntries,
  revokeInvitation,
  type InvitationForm,
} from "../invitationActions.ts";
import { usePaging } from "../paging.ts";
import { Pager, RowMenuButton } from "../parts.tsx";
import {
  useOperatorInvitations,
  useOperatorOverview,
  useOperatorRefresh,
} from "../queries.ts";
import type {
  Invitation,
  InvitationFilter,
  InvitationStatus,
  OperatorOverview,
} from "../types.ts";

const INVITATION_FILTERS = [
  "all",
  "open",
  "used",
] as const satisfies readonly InvitationFilter[];

const STATUS_TONE = {
  open: "success",
  used: "neutral",
  expired: "warning",
  revoked: "danger",
} as const satisfies Record<InvitationStatus, string>;

export function InvitationsPage() {
  const t = useT(operator);
  const overview = useOperatorOverview().data;
  const refresh = useOperatorRefresh();
  const [status, setStatus] = useState<InvitationFilter>("all");
  const [offset, setOffset] = usePaging(status);
  const query = useOperatorInvitations({
    status,
    offset: offset || undefined,
  });
  const data = query.data;

  return (
    <OperatorPageFrame page="einladungen">
      {overview ? <ModeNotice registration={overview.registration} /> : null}
      <InvitationCreator mail={overview?.mail ?? false} refresh={refresh} />
      <div className="operator-toolbar">
        <ChipGroup
          label={t("invitations.filterLabel")}
          options={INVITATION_FILTERS.map((value) => ({
            value,
            label: t(`invitations.filter.${value}`),
          }))}
          value={status}
          onChange={setStatus}
        />
      </div>
      {query.isError && !data ? (
        <LoadError
          message={t("invitations.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={4} />
      ) : data.invitations.length === 0 ? (
        <p className="operator-empty">{t(`invitations.empty.${status}`)}</p>
      ) : (
        <>
          <InvitationTable
            invitations={data.invitations}
            menu={(invitation) =>
              invitationMenuEntries(invitation, {
                copy: () => void copyInvitationLink(invitation.link),
                revoke: () => void revokeInvitation(invitation, refresh),
              })
            }
          />
          <Pager offset={offset} total={data.total} onOffset={setOffset} />
        </>
      )}
    </OperatorPageFrame>
  );
}

/** Which registration this instance runs; set by the deployment. */
export function ModeNotice({
  registration,
}: {
  registration: OperatorOverview["registration"];
}) {
  const t = useT(operator);
  return (
    <p className="operator-notice">
      <strong>{t(`invitations.mode.${registration}`)}</strong>
      {registration === "open" ? <> {t("invitations.mode.openHint")}</> : null}
    </p>
  );
}

/** The form, and the link it made last, ready to copy. */
export function InvitationCreator({
  mail,
  refresh,
}: {
  /** This instance sends mail (`GET /api/operator`). */
  mail: boolean;
  refresh: () => void;
}) {
  const t = useT(operator);
  const form = useRef<HTMLFormElement>(null);
  const [values, setValues] = useState<InvitationForm>(EMPTY_INVITATION_FORM);
  const [pending, setPending] = useState(false);
  const [created, setCreated] = useState<{
    invitation: Invitation;
    mailedTo: string | null;
  } | null>(null);
  const { errors, clearField, setFields, fromError } = useFormErrors(form);
  const offerMail = canSendMail(values, mail);

  const change = (
    field: keyof InvitationForm,
    next: Partial<InvitationForm>,
  ) => {
    setValues((current) => ({ ...current, ...next }));
    clearField(field === "sendMail" ? "send_mail" : field);
  };

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    const local = checkInvitationForm(values);
    if (Object.keys(local).length > 0) {
      setFields(invitationFieldMessages(local));
      return;
    }
    const input = invitationInput(values, mail);
    setPending(true);
    try {
      const invitation = await remote.createInvitation(input);
      setCreated({
        invitation,
        mailedTo: input.send_mail ? (invitation.email ?? null) : null,
      });
      setValues(EMPTY_INVITATION_FORM);
    } catch (error) {
      if (
        error instanceof ApiError &&
        error.code === "validation_failed" &&
        Object.keys(error.fields).length > 0
      )
        setFields(invitationFieldMessages(error.fields));
      else fromError(error);
    } finally {
      setPending(false);
      refresh();
    }
  };

  return (
    <section
      className="operator-form"
      aria-labelledby="operator-invitation-title"
    >
      <h2 id="operator-invitation-title">{t("invitations.form.title")}</h2>
      <form ref={form} onSubmit={onSubmit} noValidate>
        <div className="operator-form-fields">
          <Field
            id="operator-invitation-email"
            label={t("invitations.form.email")}
            hint={t("invitations.form.emailHint")}
            type="email"
            autoComplete="off"
            value={values.email}
            onChange={(event) =>
              change("email", {
                email: event.target.value,
                ...(event.target.value.trim() ? {} : { sendMail: false }),
              })
            }
            error={errors.fields.email}
          />
          <Field
            id="operator-invitation-note"
            label={t("invitations.form.note")}
            type="text"
            maxLength={NOTE_MAX}
            value={values.note}
            onChange={(event) => change("note", { note: event.target.value })}
            error={errors.fields.note}
          />
        </div>
        <div className="operator-form-row">
          {/* The chips carry the same name for screen readers. */}
          <span className="ui-label" aria-hidden="true">
            {t("invitations.form.expiresLabel")}
          </span>
          <ChipGroup
            label={t("invitations.form.expiresLabel")}
            options={EXPIRY_CHOICES.map((value) => ({
              value,
              label: t(`invitations.form.expires.${value}`),
            }))}
            value={values.expiry}
            onChange={(expiry) => change("expiry", { expiry })}
          />
        </div>
        {errors.fields.expires_in_days ? (
          <p className="ui-field-error">{errors.fields.expires_in_days}</p>
        ) : null}
        {offerMail ? (
          <Checkbox
            label={t("invitations.form.sendMail")}
            checked={values.sendMail}
            onChange={(event) =>
              change("sendMail", { sendMail: event.target.checked })
            }
          />
        ) : null}
        {errors.fields.send_mail ? (
          <p className="ui-field-error">{errors.fields.send_mail}</p>
        ) : null}
        <FormError message={errors.form} />
        <div className="operator-form-actions">
          <Button
            type="submit"
            variant="primary"
            pending={pending}
            pendingLabel={t("invitations.form.submit")}
          >
            {t("invitations.form.submit")}
          </Button>
        </div>
      </form>
      {created ? (
        <CreatedLink
          link={created.invitation.link}
          mailedTo={created.mailedTo}
        />
      ) : null}
    </section>
  );
}

export function CreatedLink({
  link,
  mailedTo,
}: {
  link: string;
  mailedTo: string | null;
}) {
  const t = useT(operator);
  return (
    <div className="operator-created">
      <label className="ui-label" htmlFor="operator-invitation-link">
        {t("invitations.created.linkLabel")}
      </label>
      <div className="operator-created-link">
        <input
          id="operator-invitation-link"
          className="ui-field"
          readOnly
          value={link}
          onFocus={(event) => event.currentTarget.select()}
        />
        <CopyButton variant="primary" onCopy={() => copyInvitationLink(link)}>
          {t("invitations.menu.copy")}
        </CopyButton>
      </div>
      {mailedTo ? (
        <p className="ui-hint">
          {t("invitations.created.mailed", { email: mailedTo })}
        </p>
      ) : null}
    </div>
  );
}

export function InvitationTable({
  invitations,
  menu,
}: {
  invitations: Invitation[];
  menu: (invitation: Invitation) => ReturnType<typeof invitationMenuEntries>;
}) {
  const t = useT(operator);
  const column = {
    email: t("invitations.column.email"),
    note: t("invitations.column.note"),
    createdBy: t("invitations.column.createdBy"),
    expires: t("invitations.column.expires"),
    status: t("invitations.column.status"),
  };
  return (
    <table className="operator-table operator-table--invitations">
      <thead>
        <tr>
          <th scope="col">{t("invitations.column.link")}</th>
          <th scope="col">{column.email}</th>
          <th scope="col">{column.note}</th>
          <th scope="col">{column.createdBy}</th>
          <th scope="col">{column.expires}</th>
          <th scope="col">{column.status}</th>
          <th scope="col" className="is-menu">
            <span className="sr-only">{t("invitations.column.actions")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {invitations.map((invitation) => {
          const entries = menu(invitation);
          return (
            <tr key={invitation.id}>
              <th scope="row" className="is-title is-link">
                <code className="operator-link" title={invitation.link}>
                  {invitation.link}
                </code>
              </th>
              <td data-label={column.email} className="is-email">
                {invitation.email ?? (
                  <span className="operator-muted">
                    {t("invitations.anyAddress")}
                  </span>
                )}
              </td>
              <td data-label={column.note}>{invitation.note ?? <None />}</td>
              <td data-label={column.createdBy}>
                {invitation.created_by?.name ?? (
                  <span className="operator-muted">
                    {t("invitations.creatorGone")}
                  </span>
                )}
              </td>
              <td data-label={column.expires}>
                {invitation.expires_at ? (
                  <time
                    dateTime={invitation.expires_at}
                    title={formatDate(invitation.expires_at, {
                      dateStyle: "medium",
                      timeStyle: "short",
                    })}
                  >
                    {formatWhen(invitation.expires_at)}
                  </time>
                ) : (
                  t("invitations.never")
                )}
              </td>
              <td data-label={column.status}>
                <Badge
                  tone={STATUS_TONE[invitation.status]}
                  title={
                    invitation.used_by
                      ? t("invitations.usedBy", {
                          name: invitation.used_by.name,
                        })
                      : undefined
                  }
                >
                  {t(`invitations.status.${invitation.status}`)}
                </Badge>
              </td>
              <td className="is-menu">
                {entries.length > 0 ? (
                  <RowMenuButton
                    label={t("invitations.menuLabel", {
                      code: invitation.code,
                    })}
                    entries={entries}
                  />
                ) : null}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function None() {
  const t = useT(operator);
  return (
    <>
      <span aria-hidden="true">{t("cell.none")}</span>
      <span className="sr-only">{t("cell.noneLabel")}</span>
    </>
  );
}
