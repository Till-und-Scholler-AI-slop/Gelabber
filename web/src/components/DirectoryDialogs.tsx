// The dialogs of the directory "Entdecken" (v0.10).

import { useRef, useState, type FormEvent } from "react";

import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useRequestJoin } from "../discover/queries.ts";
import { MESSAGE_MAX, validateMessage } from "../discover/rules.ts";
import type { Listing } from "../discover/types.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { discovery } from "../i18n/catalogs/discovery.ts";
import { Field } from "./Field.tsx";
import { FormError } from "./FormError.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { SubmitButton } from "./SubmitButton.tsx";

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
