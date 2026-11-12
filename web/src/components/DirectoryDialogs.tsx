// The dialogs of the directory "Entdecken" (v0.10).

import { useId, useRef, useState, type FormEvent } from "react";

import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useReportListing, useRequestJoin } from "../discover/queries.ts";
import {
  MESSAGE_MAX,
  REPORT_DETAILS_MAX,
  REPORT_REASONS,
  validateMessage,
  validateReportDetails,
} from "../discover/rules.ts";
import type { Listing, ReportReason } from "../discover/types.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { discovery } from "../i18n/catalogs/discovery.ts";
import { Field } from "./Field.tsx";
import { FormError } from "./FormError.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { SubmitButton } from "./SubmitButton.tsx";
import { notify } from "./toasts.ts";
import { fieldDescription } from "./ui/fieldDescription.ts";
import { FormField } from "./ui/FormField.tsx";
import { RadioCard } from "./ui/RadioCard.tsx";
import { Textarea } from "./ui/Textarea.tsx";

/** "Beitritt anfragen": the server's name and an optional message. */
export function RequestJoinDialog({
  listing,
  onClose,
}: {
  /** The server asked for; `null` closes the dialog. */
  listing: Listing | null;
  onClose: () => void;
}) {
  const t = useT(discovery);
  return (
    <Modal open={listing !== null} onClose={onClose} title={t("request.title")}>
      {listing ? (
        <RequestJoinForm
          key={listing.server_id}
          listing={listing}
          onClose={onClose}
        />
      ) : null}
    </Modal>
  );
}

function RequestJoinForm({
  listing,
  onClose,
}: {
  listing: Listing;
  onClose: () => void;
}) {
  const t = useT(discovery);
  const tc = useT(common);
  const form = useRef<HTMLFormElement>(null);
  const [message, setMessage] = useState("");
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const request = useRequestJoin();

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateMessage(message);
    if (code) {
      setFields({ message: fieldMessage("message", code) });
      return;
    }
    request.mutate(
      { serverId: listing.server_id, message },
      { onSuccess: onClose, onError: fromError },
    );
  };

  return (
    <form
      ref={form}
      onSubmit={onSubmit}
      noValidate
      className="flex flex-col gap-5"
    >
      <p className="discover-request-lead">
        {t("request.lead", { name: listing.name })}
      </p>
      <Field
        id="discover-request-message"
        label={t("request.message")}
        hint={t("request.messageHint")}
        type="text"
        autoComplete="off"
        autoFocus
        maxLength={MESSAGE_MAX}
        value={message}
        onChange={(event) => {
          setMessage(event.target.value);
          clearField("message");
        }}
        error={errors.fields.message}
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>{tc("cancel")}</GhostButton>
        <SubmitButton
          pending={request.isPending}
          pendingLabel={t("request.sending")}
          cooldown={cooldown}
        >
          {t("request.submit")}
        </SubmitButton>
      </div>
    </form>
  );
}

/** "Server melden": a reason, details (needed for "Sonstiges"), "Melden". */
export function ReportListingDialog({
  listing,
  onClose,
}: {
  /** The server reported; `null` closes the dialog. */
  listing: Listing | null;
  onClose: () => void;
}) {
  const t = useT(discovery);
  return (
    <Modal open={listing !== null} onClose={onClose} title={t("report.title")}>
      {listing ? (
        <ReportListingForm
          key={listing.server_id}
          listing={listing}
          onClose={onClose}
        />
      ) : null}
    </Modal>
  );
}

function ReportListingForm({
  listing,
  onClose,
}: {
  listing: Listing;
  onClose: () => void;
}) {
  const t = useT(discovery);
  const tc = useT(common);
  const form = useRef<HTMLFormElement>(null);
  const reasonName = useId();
  const reasonErrorId = useId();
  // No reason is chosen for the reporter.
  const [reason, setReason] = useState<ReportReason | null>(null);
  const [details, setDetails] = useState("");
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const report = useReportListing();
  const detailsError = errors.fields.details;
  const detailsHint = t("report.detailsHint");

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!reason) {
      setFields({ reason: t("report.reasonRequired") });
      return;
    }
    const code = validateReportDetails(details, reason);
    if (code) {
      setFields({
        details:
          code === "required"
            ? t("report.detailsRequired")
            : fieldMessage("details", code),
      });
      return;
    }
    const text = details.trim();
    report.mutate(
      {
        serverId: listing.server_id,
        input: text ? { reason, details: text } : { reason },
      },
      {
        onSuccess: () => {
          onClose();
          notify(t("report.sent"));
        },
        onError: fromError,
      },
    );
  };

  return (
    <form
      ref={form}
      onSubmit={onSubmit}
      noValidate
      className="flex flex-col gap-5"
    >
      <p className="discover-request-lead">
        {t("report.lead", { name: listing.name })}
      </p>
      <fieldset
        className="flex flex-col gap-2"
        aria-describedby={errors.fields.reason ? reasonErrorId : undefined}
      >
        <legend className="ui-label mb-2">{t("report.reason")}</legend>
        {REPORT_REASONS.map((value) => (
          <RadioCard
            key={value}
            name={reasonName}
            value={value}
            checked={reason === value}
            onChange={() => {
              setReason(value);
              clearField("details");
              clearField("reason");
            }}
            label={t(`report.reasons.${value}`)}
          />
        ))}
        {errors.fields.reason ? (
          <p id={reasonErrorId} className="ui-field-error">
            {errors.fields.reason}
          </p>
        ) : null}
      </fieldset>
      <FormField
        id="discover-report-details"
        label={t("report.details")}
        hint={detailsHint}
        error={detailsError}
      >
        <Textarea
          id="discover-report-details"
          rows={3}
          maxLength={REPORT_DETAILS_MAX}
          value={details}
          onChange={(event) => {
            setDetails(event.target.value);
            clearField("details");
          }}
          invalid={Boolean(detailsError)}
          aria-required={reason === "other" || undefined}
          aria-describedby={fieldDescription("discover-report-details", {
            error: detailsError,
            hint: detailsHint,
          })}
          className="resize-y"
        />
      </FormField>
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>{tc("cancel")}</GhostButton>
        <SubmitButton
          pending={report.isPending}
          pendingLabel={t("report.sending")}
          cooldown={cooldown}
        >
          {t("report.submit")}
        </SubmitButton>
      </div>
    </form>
  );
}
