// Dialogs for adding a server (create or join), creating/renaming a
// category, and creating/editing a channel. Channel and category mutations
// are optimistic (see `servers/queries.ts`), so those dialogs close the
// moment the form is valid.

import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState, type FormEvent } from "react";

import { currentUserId, useUserId } from "../auth/scope.ts";
import { fieldMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { servers } from "../i18n/catalogs/servers.ts";
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
import { channelRequest } from "../servers/channelRequest.ts";
import {
  accessOf,
  sameAccess,
  type AccessSelection,
} from "../servers/roles.ts";
import type {
  Category,
  Channel,
  ChannelKind,
  ServerDetail,
} from "../servers/types.ts";
import { ChannelAccessField } from "./ChannelAccessField.tsx";
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
  const t = useT(servers);
  const [step, setStep] = useState<"choose" | "create" | "join">("choose");
  // After "Zurück", focus returns to the choice that was taken.
  const [from, setFrom] = useState<"create" | "join" | null>(null);
  const [dirty, setDirty] = useState(false);
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
      ? t("create.title")
      : step === "join"
        ? t("join.title")
        : t("add.title");
  return (
    <Modal open={open} onClose={close} title={title} dirty={dirty}>
      {!open ? null : step === "create" ? (
        <CreateServerForm onClose={close} onBack={back} onDirty={setDirty} />
      ) : step === "join" ? (
        <JoinServerForm
          autoFocus
          onJoin={close}
          onBack={back}
          onDirty={setDirty}
        />
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
              <strong>{t("create.title")}</strong>
              <small>{t("create.hint")}</small>
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
              <strong>{t("join.title")}</strong>
              <small>{t("join.hint")}</small>
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
  const t = useT(servers);
  const [dirty, setDirty] = useState(false);
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("create.title")}
      dirty={dirty}
    >
      {open ? <CreateServerForm onClose={onClose} onDirty={setDirty} /> : null}
    </Modal>
  );
}

/** Tells the dialog around a form whether closing it would lose typed
 *  text: the backdrop then leaves it open, and Esc asks first. */
function useReportDirty(
  onDirty: ((dirty: boolean) => void) | undefined,
  dirty: boolean,
) {
  useEffect(() => onDirty?.(dirty), [onDirty, dirty]);
  useEffect(() => () => onDirty?.(false), [onDirty]);
}

/**
 * Creates the server, opens its overview and offers an invite link right
 * away: a server is only useful once people are in it.
 */
function CreateServerForm({
  onClose,
  onBack,
  onDirty,
}: {
  onClose: () => void;
  onBack?: () => void;
  onDirty?: (dirty: boolean) => void;
}) {
  const t = useT(servers);
  const tc = useT(common);
  const navigate = useNavigate();
  const userId = useUserId();
  const remember = useLastChannel((s) => s.remember);
  const [name, setName] = useState("");
  useReportDirty(onDirty, name !== "");
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
        label={t("form.nameLabel")}
        type="text"
        autoFocus
        maxLength={100}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          clearField("name");
        }}
        error={errors.fields.name}
        hint={t("create.hint")}
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onBack ?? onClose}>
          {onBack ? tc("back") : tc("cancel")}
        </GhostButton>
        <SubmitButton
          pending={mutation.isPending}
          pendingLabel={t("form.creating")}
        >
          {t("form.create")}
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
  onDirty,
  inline = false,
}: {
  autoFocus?: boolean;
  onJoin?: () => void;
  onBack?: () => void;
  onDirty?: (dirty: boolean) => void;
  /** Field and button side by side, without a dialog's button row. */
  inline?: boolean;
}) {
  const t = useT(servers);
  const tc = useT(common);
  const navigate = useNavigate();
  const [value, setValue] = useState("");
  useReportDirty(onDirty, value !== "");
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
      label={t("join.inputLabel")}
      type="text"
      autoComplete="off"
      autoCapitalize="off"
      spellCheck={false}
      autoFocus={autoFocus}
      placeholder={t("join.inputPlaceholder")}
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
        <Button type="submit">{t("join.submit")}</Button>
      </form>
    );
  }
  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      {field}
      <div className="flex justify-end gap-2">
        {onBack ? (
          <GhostButton onClick={onBack}>{tc("back")}</GhostButton>
        ) : null}
        <Button type="submit" variant="primary">
          {tc("next")}
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
  const t = useT(servers);
  const title =
    state?.mode === "edit"
      ? t("category.renameTitle")
      : t("category.createTitle");
  const [dirty, setDirty] = useState(false);
  return (
    <Modal open={state !== null} onClose={onClose} title={title} dirty={dirty}>
      {state ? (
        <CategoryForm
          server={server}
          state={state}
          onClose={onClose}
          onDirty={setDirty}
        />
      ) : null}
    </Modal>
  );
}

function CategoryForm({
  server,
  state,
  onClose,
  onDirty,
}: {
  server: ServerDetail;
  state: CategoryDialogState;
  onClose: () => void;
  onDirty: (dirty: boolean) => void;
}) {
  const t = useT(servers);
  const tc = useT(common);
  const [initialName] = useState(
    state.mode === "edit" ? state.category.name : "",
  );
  const [name, setName] = useState(initialName);
  useReportDirty(onDirty, name !== initialName);
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
        label={t("form.nameLabel")}
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
        <GhostButton onClick={onClose}>{tc("cancel")}</GhostButton>
        <SubmitButton pending={false} pendingLabel="">
          {state.mode === "edit" ? tc("save") : t("form.create")}
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
  const t = useT(servers);
  const title =
    state?.mode === "edit" ? t("channel.editTitle") : t("channel.createTitle");
  const [dirty, setDirty] = useState(false);
  return (
    <Modal open={state !== null} onClose={onClose} title={title} dirty={dirty}>
      {state ? (
        <ChannelForm
          server={server}
          state={state}
          onClose={onClose}
          onDirty={setDirty}
        />
      ) : null}
    </Modal>
  );
}

function ChannelForm({
  server,
  state,
  onClose,
  onDirty,
}: {
  server: ServerDetail;
  state: ChannelDialogState;
  onClose: () => void;
  onDirty: (dirty: boolean) => void;
}) {
  const t = useT(servers);
  const tc = useT(common);
  const editing = state.mode === "edit" ? state.channel : null;
  const [initialName] = useState(editing?.name ?? "");
  const [name, setName] = useState(initialName);
  // "Alle Mitglieder" (`null`) unless the channel is limited.
  const [initialAccess] = useState<AccessSelection>(
    editing ? accessOf(editing) : null,
  );
  const [access, setAccess] = useState(initialAccess);
  useReportDirty(
    onDirty,
    name !== initialName || !sameAccess(access, initialAccess),
  );
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
    const request = channelRequest(editing, {
      name,
      kind,
      categoryId: parentId,
      initialAccess,
      access,
      roles: server.roles,
    });
    if (request.mode === "edit") {
      update.mutate({ id: request.id, patch: request.patch });
    } else {
      create.mutate(request.input);
    }
    onClose();
  };

  return (
    <form onSubmit={onSubmit} noValidate className="flex flex-col gap-5">
      {!editing ? (
        <fieldset className="flex flex-col gap-2">
          <legend className="ui-label mb-2">{t("channel.kindLegend")}</legend>
          <div className="grid grid-cols-2 gap-2">
            {(
              [
                ["text", t("channel.kindText"), t("channel.kindTextHint")],
                ["voice", t("channel.kindVoice"), t("channel.kindVoiceHint")],
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
        label={t("form.nameLabel")}
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
            ? t("channel.savedAs", { name: preview })
            : undefined
        }
      />
      <FormField id="channel-category" label={t("channel.categoryLabel")}>
        <Select
          id="channel-category"
          value={parentId}
          onChange={(event) => setCategoryId(event.target.value)}
        >
          <option value="">{t("channel.noCategory")}</option>
          {categories.map((category) => (
            <option key={category.id} value={category.id}>
              {category.name}
            </option>
          ))}
        </Select>
      </FormField>
      <ChannelAccessField server={server} value={access} onChange={setAccess} />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>{tc("cancel")}</GhostButton>
        <SubmitButton pending={false} pendingLabel="">
          {editing ? tc("save") : t("form.create")}
        </SubmitButton>
      </div>
    </form>
  );
}
