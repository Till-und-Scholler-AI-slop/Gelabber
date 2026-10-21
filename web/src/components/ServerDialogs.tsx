// Dialogs for adding a server (create or join), creating/renaming a
// category, and creating/editing a channel. Channel and category mutations
// are optimistic (see `servers/queries.ts`), so those dialogs close the
// moment the form is valid.

import { useNavigate } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { currentUserId, useUserId } from "../auth/scope.ts";
import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { inviteInputMessage, parseInvite } from "../servers/inviteCode.ts";
import { promptInvite } from "../servers/invitePrompt.ts";
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
import { HashIcon, LinkIcon, PlusIcon, SpeakerIcon } from "./Icons.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { SubmitButton } from "./SubmitButton.tsx";
import { Button } from "./ui/Button.tsx";
import { FormField } from "./ui/FormField.tsx";
import { RadioCard } from "./ui/RadioCard.tsx";
import { Select } from "./ui/Select.tsx";
import "./invite.css";

// ---------------------------------------------------------------------------

/** The rail's "+": make a server of your own, or join one by invite. */
export function AddServerDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [step, setStep] = useState<"choose" | "create" | "join">("choose");
  // After "Zurück", focus returns to the choice that was taken.
  const [from, setFrom] = useState<"create" | "join" | null>(null);
  const close = () => {
    onClose();
    setStep("choose");
    setFrom(null);
  };
  const back = () => {
    setFrom(step === "choose" ? null : step);
    setStep("choose");
  };
  const title =
    step === "create"
      ? "Server erstellen"
      : step === "join"
        ? "Server beitreten"
        : "Server hinzufügen";
  return (
    <Modal open={open} onClose={close} title={title}>
      {!open ? null : step === "create" ? (
        <CreateServerForm onClose={close} onBack={back} />
      ) : step === "join" ? (
        <JoinServerForm autoFocus onJoin={close} onBack={back} />
      ) : (
        <div className="add-server-choices">
          <button
            type="button"
            className="add-server-choice"
            autoFocus={from === "create"}
            onClick={() => setStep("create")}
          >
            <PlusIcon size={20} />
            <span>
              <strong>Server erstellen</strong>
              <small>{CREATE_HINT}</small>
            </span>
          </button>
          <button
            type="button"
            className="add-server-choice"
            autoFocus={from === "join"}
            onClick={() => setStep("join")}
          >
            <LinkIcon size={20} />
            <span>
              <strong>Server beitreten</strong>
              <small>Mit einem Einladungslink oder Code.</small>
            </span>
          </button>
        </div>
      )}
    </Modal>
  );
}

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

const CREATE_HINT =
  "Du verwaltest den Server. #allgemein und ein Sprachraum sind gleich da.";

/**
 * Creates the server, opens its overview and offers an invite link right
 * away: a server is only useful once people are in it.
 */
function CreateServerForm({
  onClose,
  onBack,
}: {
  onClose: () => void;
  onBack?: () => void;
}) {
  const navigate = useNavigate();
  const userId = useUserId();
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
        if (!userId || currentUserId() !== userId) return;
        const first = detail.channels.find((c) => c.kind === "text");
        if (first) remember(userId, detail.id, first.id);
        onClose();
        void navigate({ to: "/s/$serverId", params: { serverId: detail.id } });
        promptInvite({ userId, id: detail.id, name: detail.name });
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
        hint={CREATE_HINT}
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onBack ?? onClose}>
          {onBack ? "Zurück" : "Abbrechen"}
        </GhostButton>
        <SubmitButton pending={mutation.isPending} pendingLabel="Erstellen…">
          Erstellen
        </SubmitButton>
      </div>
    </form>
  );
}

/**
 * "Einladungslink oder Code": takes whatever was pasted and opens the
 * invite page, which shows the server before anyone joins. The desktop app
 * and an installed PWA have no address bar, so this is their way in.
 */
export function JoinServerForm({
  autoFocus,
  onJoin,
  onBack,
  inline = false,
}: {
  autoFocus?: boolean;
  onJoin?: () => void;
  onBack?: () => void;
  /** Field and button side by side, without a dialog's button row. */
  inline?: boolean;
}) {
  const navigate = useNavigate();
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | undefined>();

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const parsed = parseInvite(value, window.location.origin);
    if ("error" in parsed) {
      setError(inviteInputMessage(parsed.error));
      event.currentTarget.querySelector<HTMLInputElement>("input")?.focus();
      return;
    }
    onJoin?.();
    void navigate({ to: "/invite/$code", params: { code: parsed.code } });
  };

  const field = (
    <Field
      id={inline ? "join-invite-inline" : "join-invite"}
      label="Einladungslink oder Code"
      type="text"
      autoComplete="off"
      autoCapitalize="off"
      spellCheck={false}
      autoFocus={autoFocus}
      placeholder="Link oder Code einfügen"
      value={value}
      onChange={(event) => {
        setValue(event.target.value);
        setError(undefined);
      }}
      error={error}
    />
  );

  if (inline) {
    return (
      <form onSubmit={onSubmit} noValidate className="join-server-inline">
        {field}
        <Button type="submit">Beitreten</Button>
      </form>
    );
  }
  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      {field}
      <div className="flex justify-end gap-2">
        {onBack ? <GhostButton onClick={onBack}>Zurück</GhostButton> : null}
        <Button type="submit" variant="primary">
          Weiter
        </Button>
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
          <legend className="ui-label mb-2">Kanaltyp</legend>
          <div className="grid grid-cols-2 gap-2">
            {(
              [
                ["text", "Text", "Nachrichten, Dateien"],
                ["voice", "Voice", "Sprache, Go Live"],
              ] as const
            ).map(([value, label, hint]) => (
              <RadioCard
                key={value}
                name="kind"
                value={value}
                checked={kind === value}
                onChange={() => setKind(value)}
                label={
                  <span className="flex items-center gap-1.5">
                    {value === "text" ? (
                      <HashIcon size={14} />
                    ) : (
                      <SpeakerIcon size={14} />
                    )}
                    {label}
                  </span>
                }
                description={hint}
              />
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
      <FormField id="channel-category" label="Kategorie">
        <Select
          id="channel-category"
          value={parentId}
          onChange={(event) => setCategoryId(event.target.value)}
        >
          <option value="">Ohne Kategorie</option>
          {categories.map((category) => (
            <option key={category.id} value={category.id}>
              {category.name}
            </option>
          ))}
        </Select>
      </FormField>
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
