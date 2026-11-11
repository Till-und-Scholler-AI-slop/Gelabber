// Pieces the operator pages share: the search field, the pager, a server's
// tile, the "…" row menu and the dialog that asks for a reason or a note.

import {
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type ReactNode,
} from "react";

import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { MoreIcon, SearchIcon } from "../components/Icons.tsx";
import { GhostButton, Modal } from "../components/Modal.tsx";
import { RowMenu } from "../components/RowMenu.tsx";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { tileInitials, tileTone } from "../components/initials.ts";
import { Button } from "../components/ui/Button.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Input } from "../components/ui/Input.tsx";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import { SEARCH_MAX } from "./areas.ts";
import "./operator.css";

/** The API answers lists 50 rows at a time. */
export const PAGE_SIZE = 50;

/** Typing waits this long before it asks: every list call is audited. */
export const SEARCH_DEBOUNCE_MS = 300;

export function SearchField({
  label,
  value,
  onChange,
}: {
  /** Also the placeholder, as in the reference. */
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="operator-search">
      <SearchIcon size={16} aria-hidden="true" />
      <Input
        type="search"
        aria-label={label}
        placeholder={label}
        maxLength={SEARCH_MAX}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}

/** "1–50 von 120" with back and next; nothing while one page holds all. */
export function Pager({
  offset,
  total,
  onOffset,
}: {
  offset: number;
  total: number;
  onOffset: (offset: number) => void;
}) {
  const t = useT(operator);
  const tc = useT(common);
  if (total <= PAGE_SIZE) return null;
  return (
    <nav className="operator-pager" aria-label={t("pager.label")}>
      <span>
        {t("pager.range", {
          from: offset + 1,
          to: Math.min(offset + PAGE_SIZE, total),
          total,
        })}
      </span>
      <Button
        size="sm"
        disabled={offset === 0}
        onClick={() => onOffset(Math.max(0, offset - PAGE_SIZE))}
      >
        {tc("back")}
      </Button>
      <Button
        size="sm"
        disabled={offset + PAGE_SIZE >= total}
        onClick={() => onOffset(offset + PAGE_SIZE)}
      >
        {tc("next")}
      </Button>
    </nav>
  );
}

/** A server's small tile and name, as in the rail but square. */
export function ServerName({
  server,
  badge,
}: {
  server: { id: string; name: string; icon_url: string | null };
  badge?: ReactNode;
}) {
  return (
    <span className="operator-server">
      <span
        aria-hidden="true"
        className="operator-tile"
        style={
          {
            "--lr-tile": `var(--lr-tile-${tileTone(server.id)})`,
          } as CSSProperties
        }
      >
        <ServerIconFace
          url={server.icon_url}
          fallback={tileInitials(server.name)}
        />
      </span>
      <span className="operator-server-name">{server.name}</span>
      {badge}
    </span>
  );
}

/** The "…" button of a row and its menu. */
export function RowMenuButton({
  label,
  entries,
}: {
  /** "Aktionen für Filmclub". */
  label: string;
  entries: MenuEntry[];
}) {
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <IconButton
        ref={button}
        label={label}
        icon={<MoreIcon size={18} />}
        size="sm"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      />
      {open ? (
        <RowMenu
          open
          onClose={() => setOpen(false)}
          anchor={button}
          label={label}
          entries={entries}
        />
      ) : null}
    </>
  );
}

/** The longest reason or note the API takes. */
export const REASON_MAX = 500;

export type ReasonRequest = {
  title: string;
  message?: string;
  /** The field's label. */
  label: string;
  /** Which API field it is (`reason`, `note`), so its errors land on it. */
  field: string;
  required: boolean;
  confirmLabel: string;
  danger?: boolean;
  /** Sends it; a throw keeps the dialog open with the error in it. */
  submit: (text: string) => Promise<unknown>;
};

/**
 * Asks for a reason (blocking a server) or a note (closing a report) and
 * sends it. One line: the API refuses line breaks in both.
 */
export function ReasonDialog({
  request,
  onClose,
}: {
  request: ReasonRequest | null;
  onClose: () => void;
}) {
  const [dirty, setDirty] = useState(false);
  const close = () => {
    setDirty(false);
    onClose();
  };
  return (
    <Modal
      open={request !== null}
      onClose={close}
      title={request?.title ?? ""}
      dirty={dirty}
    >
      {request ? (
        <ReasonForm request={request} onDirty={setDirty} onClose={close} />
      ) : null}
    </Modal>
  );
}

function ReasonForm({
  request,
  onDirty,
  onClose,
}: {
  request: ReasonRequest;
  onDirty: (dirty: boolean) => void;
  onClose: () => void;
}) {
  const t = useT(operator);
  const tc = useT(common);
  const form = useRef<HTMLFormElement>(null);
  const [text, setText] = useState("");
  const [pending, setPending] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors(form);
  const id = `operator-${request.field}`;

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    const value = text.trim();
    if (request.required && !value) {
      setFields({ [request.field]: t("dialogs.reasonRequired") });
      return;
    }
    setPending(true);
    try {
      await request.submit(value);
      onClose();
    } catch (error) {
      fromError(error);
    } finally {
      setPending(false);
    }
  };

  return (
    <form
      ref={form}
      onSubmit={onSubmit}
      noValidate
      className="flex flex-col gap-5"
    >
      {request.message ? (
        <p className="text-lr-muted">{request.message}</p>
      ) : null}
      <Field
        id={id}
        label={request.label}
        type="text"
        autoFocus
        required={request.required}
        maxLength={REASON_MAX}
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          onDirty(event.target.value.trim().length > 0);
          clearField(request.field);
        }}
        error={errors.fields[request.field]}
      />
      <FormError message={errors.form} />
      <div className="flex justify-end gap-2">
        <GhostButton onClick={onClose}>{tc("cancel")}</GhostButton>
        <Button
          type="submit"
          variant={request.danger ? "danger" : "primary"}
          pending={pending}
          pendingLabel={request.confirmLabel}
        >
          {request.confirmLabel}
        </Button>
      </div>
    </form>
  );
}
