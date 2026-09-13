// Dialogs for creating a server, creating/renaming a category, and
// creating/editing a channel. All mutations are optimistic (see
// `servers/queries.ts`), so the dialog closes the moment the form is valid.

import { useNavigate } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useLastChannel } from "../servers/lastChannel.ts";
import {
  isPendingId,
  useCreateCategory,
  useCreateChannel,
  useCreateServer,
  useUpdateCategory,
  useUpdateChannel,
} from "../servers/queries.ts";
import {
  slugifyChannelName,
  validateChannelName,
  validateName,
} from "../servers/rules.ts";
import type {
  Category,
  Channel,
  ChannelKind,
  ServerDetail,
} from "../servers/types.ts";
import { Field } from "./Field.tsx";
import { FormError } from "./FormError.tsx";
import { HashIcon, SpeakerIcon } from "./Icons.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { SubmitButton } from "./SubmitButton.tsx";

// ---------------------------------------------------------------------------

export function CreateServerDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Modal open={open} onClose={onClose} title="Server erstellen">
      {open ? <CreateServerForm onClose={onClose} /> : null}
    </Modal>
  );
}

function CreateServerForm({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const remember = useLastChannel((s) => s.remember);
  const [name, setName] = useState("");
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const mutation = useCreateServer();

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateName(name);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    mutation.mutate(name, {
      onSuccess: (detail) => {
        const first = detail.channels.find((c) => c.kind === "text");
        if (first) remember(detail.id, first.id);
        onClose();
        void navigate(
          first
            ? {
                to: "/s/$serverId/c/$channelId",
                params: { serverId: detail.id, channelId: first.id },
              }
            : { to: "/s/$serverId", params: { serverId: detail.id } },
        );
      },
      onError: fromError,
    });
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      <Field
        id="server-name"
        label="Name"
        type="text"
        autoFocus
        maxLength={100}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          clearField("name");
        }}
        error={errors.fields.name}
        hint="Du bist Owner. Ein Kanal #allgemein ist sofort da."
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>Abbrechen</GhostButton>
        <SubmitButton pending={mutation.isPending} pendingLabel="Erstellen…">
          Erstellen
        </SubmitButton>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------------------

export type CategoryDialogState =
  { mode: "create" } | { mode: "edit"; category: Category };

export function CategoryDialog({
  server,
  state,
  onClose,
}: {
  server: ServerDetail;
  state: CategoryDialogState | null;
  onClose: () => void;
}) {
  const title =
    state?.mode === "edit" ? "Kategorie umbenennen" : "Kategorie erstellen";
  return (
    <Modal open={state !== null} onClose={onClose} title={title}>
      {state ? (
        <CategoryForm server={server} state={state} onClose={onClose} />
      ) : null}
    </Modal>
  );
}

function CategoryForm({
  server,
  state,
  onClose,
}: {
  server: ServerDetail;
  state: CategoryDialogState;
  onClose: () => void;
}) {
  const [name, setName] = useState(
    state.mode === "edit" ? state.category.name : "",
  );
  const { errors, clearField, setFields } = useFormErrors();
  const create = useCreateCategory(server.id);
  const update = useUpdateCategory(server.id);

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateName(name);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    // Optimistic: the row is already in the sidebar; errors roll it back.
    if (state.mode === "edit") {
      update.mutate({ id: state.category.id, name });
    } else {
      create.mutate(name);
    }
    onClose();
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      <Field
        id="category-name"
        label="Name"
        type="text"
        autoFocus
        maxLength={100}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          clearField("name");
        }}
        error={errors.fields.name}
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>Abbrechen</GhostButton>
        <SubmitButton pending={false} pendingLabel="">
          {state.mode === "edit" ? "Speichern" : "Erstellen"}
        </SubmitButton>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------------------

export type ChannelDialogState =
  | { mode: "create"; categoryId: string | null; kind?: ChannelKind }
  | { mode: "edit"; channel: Channel };

export function ChannelDialog({
  server,
  state,
  onClose,
}: {
  server: ServerDetail;
  state: ChannelDialogState | null;
  onClose: () => void;
}) {
  const title = state?.mode === "edit" ? "Kanal bearbeiten" : "Kanal erstellen";
  return (
    <Modal open={state !== null} onClose={onClose} title={title}>
      {state ? (
        <ChannelForm server={server} state={state} onClose={onClose} />
      ) : null}
    </Modal>
  );
}

function ChannelForm({
  server,
  state,
  onClose,
}: {
  server: ServerDetail;
  state: ChannelDialogState;
  onClose: () => void;
}) {
  const editing = state.mode === "edit" ? state.channel : null;
  const [name, setName] = useState(editing?.name ?? "");
  const [kind, setKind] = useState<ChannelKind>(
    state.mode === "edit" ? state.channel.kind : (state.kind ?? "text"),
  );
  const [categoryId, setCategoryId] = useState<string>(
    (state.mode === "edit" ? state.channel.category_id : state.categoryId) ??
      "",
  );
  const { errors, clearField, setFields } = useFormErrors();
  const create = useCreateChannel(server.id);
  const update = useUpdateChannel(server.id);

  // Only categories the API knows about can be a parent: a `tmp:` row from an
  // optimistic create has no real id yet, and a category deleted while this
  // dialog was open is gone. Anything else falls back to "Ohne Kategorie".
  const categories = server.categories.filter((c) => !isPendingId(c.id));
  const parentId = categories.some((c) => c.id === categoryId)
    ? categoryId
    : "";

  const preview = kind === "text" ? slugifyChannelName(name) : name.trim();

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateChannelName(name, kind);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    if (editing) {
      update.mutate({
        id: editing.id,
        patch: { name, category_id: parentId || null },
      });
    } else {
      create.mutate({ name, kind, category_id: parentId || null });
    }
    onClose();
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      {!editing ? (
        <fieldset className="flex flex-col gap-2">
          <legend className="text-sm font-medium text-neutral-800">
            Kanaltyp
          </legend>
          <div className="grid grid-cols-2 gap-2">
            {(
              [
                ["text", "Text", "Nachrichten, Dateien"],
                ["voice", "Voice", "Sprache, Go Live"],
              ] as const
            ).map(([value, label, hint]) => (
              <label
                key={value}
                className={[
                  "flex cursor-pointer flex-col gap-0.5 rounded-lg border px-3 py-2 text-sm transition",
                  kind === value
                    ? "border-neutral-900 bg-neutral-900 text-white"
                    : "border-neutral-300 hover:bg-neutral-50",
                ].join(" ")}
              >
                <input
                  type="radio"
                  name="kind"
                  value={value}
                  checked={kind === value}
                  onChange={() => setKind(value)}
                  className="sr-only"
                />
                <span className="flex items-center gap-1.5 font-medium">
                  {value === "text" ? (
                    <HashIcon size={14} />
                  ) : (
                    <SpeakerIcon size={14} />
                  )}
                  {label}
                </span>
                <span
                  className={
                    kind === value ? "text-neutral-300" : "text-neutral-500"
                  }
                >
                  {hint}
                </span>
              </label>
            ))}
          </div>
        </fieldset>
      ) : null}
      <Field
        id="channel-name"
        label="Name"
        type="text"
        autoFocus
        maxLength={100}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          clearField("name");
        }}
        error={errors.fields.name}
        hint={
          kind === "text" && preview && preview !== name.trim()
            ? `Wird gespeichert als #${preview}`
            : undefined
        }
      />
      <div className="flex flex-col gap-1.5">
        <label
          htmlFor="channel-category"
          className="text-sm font-medium text-neutral-800"
        >
          Kategorie
        </label>
        <select
          id="channel-category"
          value={parentId}
          onChange={(event) => setCategoryId(event.target.value)}
          className="rounded-lg border border-neutral-300 bg-white px-3 py-2 text-base text-neutral-900 outline-none transition focus:border-neutral-500 focus:ring-2 focus:ring-neutral-200"
        >
          <option value="">Ohne Kategorie</option>
          {categories.map((category) => (
            <option key={category.id} value={category.id}>
              {category.name}
            </option>
          ))}
        </select>
      </div>
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>Abbrechen</GhostButton>
        <SubmitButton pending={false} pendingLabel="">
          {editing ? "Speichern" : "Erstellen"}
        </SubmitButton>
      </div>
    </form>
  );
}
