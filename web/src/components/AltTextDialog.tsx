// "Bild beschreiben" (#575): the alt text of one picture, written before
// sending (composer) or later by its author (message list).

import { useId, useState } from "react";

import {
  ALT_TEXT_MAX,
  altTextLength,
  withoutControls,
} from "../messages/altText.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { common } from "../i18n/catalogs/common.ts";
import { Modal } from "./Modal.tsx";
import { Button } from "./ui/Button.tsx";
import { Textarea } from "./ui/Textarea.tsx";

export function AltTextDialog({
  open,
  name,
  preview,
  initial,
  pending = false,
  onSave,
  onClose,
}: {
  open: boolean;
  /** The picture's file name, for the label. */
  name: string;
  /** Shown above the field, so the text describes what is there. */
  preview?: string;
  initial: string;
  pending?: boolean;
  onSave: (text: string) => void;
  onClose: () => void;
}) {
  const t = useT(chat);
  const tc = useT(common);
  const ids = useId();
  const [text, setText] = useState(() => withoutControls(initial));
  const count = altTextLength(text);
  const tooLong = count > ALT_TEXT_MAX;
  const formId = `${ids}-form`;
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("altText.title")}
      dirty={text.trim() !== initial.trim()}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {tc("cancel")}
          </Button>
          <Button
            type="submit"
            form={formId}
            variant="primary"
            disabled={tooLong}
            pending={pending}
            pendingLabel={t("altText.saving")}
          >
            {tc("save")}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className="lr-alt-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (!tooLong) onSave(text);
        }}
      >
        {preview ? (
          <img className="lr-alt-preview" src={preview} alt="" />
        ) : null}
        <label htmlFor={`${ids}-text`}>{t("altText.label", { name })}</label>
        <p id={`${ids}-hint`} className="text-lr-muted">
          {t("altText.hint")}
        </p>
        <Textarea
          id={`${ids}-text`}
          value={text}
          rows={4}
          autoFocus
          invalid={tooLong}
          aria-describedby={`${ids}-hint ${ids}-count`}
          onChange={(event) => setText(withoutControls(event.target.value))}
          onKeyDown={(event) => {
            // Ctrl/Cmd+Enter saves; Enter alone is a line break.
            if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
              event.preventDefault();
              if (!tooLong) onSave(text);
            }
          }}
        />
        <p
          id={`${ids}-count`}
          className={tooLong ? "lr-alt-count is-over" : "lr-alt-count"}
          aria-live={tooLong ? "polite" : "off"}
        >
          {tooLong
            ? t("altText.tooLong", { max: String(ALT_TEXT_MAX) })
            : t("altText.count", {
                count: String(count),
                max: String(ALT_TEXT_MAX),
              })}
        </p>
      </form>
    </Modal>
  );
}
