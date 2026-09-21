// "Leute einladen": creates a link and puts it on the clipboard. Any member
// may do this; limits are optional.

import { useState, type FormEvent } from "react";

import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { inviteUrl } from "../servers/api.ts";
import { copyText } from "../servers/clipboard.ts";
import { useCreateInvite } from "../servers/queries.ts";
import { validateExpiresInHours, validateMaxUses } from "../servers/rules.ts";
import type { Server } from "../servers/types.ts";
import { Field } from "./Field.tsx";
import { FormError } from "./FormError.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { SubmitButton } from "./SubmitButton.tsx";
import { notify } from "./toasts.ts";

export function InviteDialog({
  server,
  open,
  onClose,
}: {
  server: Server;
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Modal open={open} onClose={onClose} title={`Einladen zu ${server.name}`}>
      {open ? <InviteForm server={server} onClose={onClose} /> : null}
    </Modal>
  );
}

function InviteForm({
  server,
  onClose,
}: {
  server: Server;
  onClose: () => void;
}) {
  const [maxUses, setMaxUses] = useState("");
  const [expiresIn, setExpiresIn] = useState("");
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const create = useCreateInvite(server.id);

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    const usesCode = validateMaxUses(maxUses);
    if (usesCode) fields.max_uses = fieldMessage("max_uses", usesCode);
    const hoursCode = validateExpiresInHours(expiresIn);
    if (hoursCode)
      fields.expires_in_hours = fieldMessage("expires_in_hours", hoursCode);
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    create.mutate(
      {
        ...(maxUses.trim() ? { max_uses: Number(maxUses) } : {}),
        ...(expiresIn.trim() ? { expires_in_hours: Number(expiresIn) } : {}),
      },
      {
        onSuccess: async (invite) => {
          const url = inviteUrl(invite.code);
          setLink(url);
          const ok = await copyText(url);
          setCopied(ok);
          if (ok) notify("Einladungslink kopiert.");
        },
        onError: fromError,
      },
    );
  };

  if (link) {
    return (
      <div className="flex flex-col gap-4">
        <p className="text-sm text-neutral-600 dark:text-neutral-400">
          Wer den Link öffnet und angemeldet ist, landet in{" "}
          <span className="font-medium text-neutral-900 dark:text-neutral-100">
            {server.name}
          </span>
          .
        </p>
        <div className="flex gap-2">
          <input
            readOnly
            value={link}
            onFocus={(event) => event.currentTarget.select()}
            aria-label="Einladungslink"
            className="min-w-0 flex-1 rounded-lg border border-neutral-300 dark:border-neutral-600 bg-neutral-50 dark:bg-neutral-950 px-3 py-2 font-mono text-sm"
          />
          <button
            type="button"
            onClick={async () => {
              const ok = await copyText(link);
              setCopied(ok);
            }}
            className="rounded-lg bg-neutral-900 dark:bg-neutral-700 px-3 py-2 text-sm font-medium text-white transition hover:bg-neutral-700 dark:hover:bg-neutral-600"
          >
            {copied ? "Kopiert" : "Kopieren"}
          </button>
        </div>
        <div className="flex justify-end">
          <GhostButton onClick={onClose}>Fertig</GhostButton>
        </div>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      <div className="grid grid-cols-2 gap-4">
        <Field
          id="invite-max-uses"
          label="Max. Nutzungen"
          type="number"
          inputMode="numeric"
          min={1}
          placeholder="unbegrenzt"
          value={maxUses}
          onChange={(event) => {
            setMaxUses(event.target.value);
            clearField("max_uses");
          }}
          error={errors.fields.max_uses}
        />
        <Field
          id="invite-expires"
          label="Gültig (Stunden)"
          type="number"
          inputMode="numeric"
          min={1}
          placeholder="unbegrenzt"
          value={expiresIn}
          onChange={(event) => {
            setExpiresIn(event.target.value);
            clearField("expires_in_hours");
          }}
          error={errors.fields.expires_in_hours}
        />
      </div>
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>Abbrechen</GhostButton>
        <SubmitButton pending={create.isPending} pendingLabel="Erstellen…">
          Link erstellen
        </SubmitButton>
      </div>
    </form>
  );
}
