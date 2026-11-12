// Betrieb → Feedback: the filters, how a sender and an author are named,
// what a reply will do, and the actions (done, reopen, delete). Kept apart
// from the page so tests can call them directly.

import { OS_NAMES } from "../auth/devices.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { notify, notifyError } from "../components/toasts.ts";
import type { FeedbackKind } from "../feedback/types.ts";
import { translate } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { language } from "../i18n/catalogs/language.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import * as remote from "./api.ts";
import { askThenRun } from "./serverActions.tsx";
import type {
  FeedbackListStatus,
  FeedbackQuery,
  OperatorFeedback,
  OperatorFeedbackMessage,
} from "./types.ts";

export const FEEDBACK_STATUSES = [
  "open",
  "done",
] as const satisfies readonly FeedbackListStatus[];

export type FeedbackKindFilter = "all" | FeedbackKind;

export const FEEDBACK_KIND_FILTERS = [
  "all",
  "idea",
  "problem",
  "praise",
] as const satisfies readonly FeedbackKindFilter[];

/** What the list asks for: `all` sends no kind, the first page no offset. */
export function feedbackQuery(
  status: FeedbackListStatus,
  kind: FeedbackKindFilter,
  offset: number,
): FeedbackQuery {
  return {
    status,
    kind: kind === "all" ? undefined : kind,
    offset: offset || undefined,
  };
}

/** The sender's name: the account's, "Besucher", or "Gelöschtes Konto". */
export function senderName(entry: Pick<OperatorFeedback, "sender">): string {
  switch (entry.sender.type) {
    case "account":
      return entry.sender.name;
    case "visitor":
      return translate(operator, "feedback.visitor");
    case "deleted":
      return translate(operator, "feedback.deleted");
  }
}

/** The sender's address, when there is one. */
export function senderEmail(
  entry: Pick<OperatorFeedback, "sender">,
): string | null {
  return entry.sender.type === "deleted" ? null : entry.sender.email;
}

/**
 * Who wrote a message of the thread. Operators see each other's names; the
 * sender's own messages carry the sender's name, or "Besucher".
 */
export function authorName(
  entry: Pick<OperatorFeedback, "sender">,
  message: Pick<OperatorFeedbackMessage, "from" | "author">,
): string {
  if (message.from === "person" && entry.sender.type === "visitor")
    return translate(operator, "feedback.visitor");
  if (message.author) return message.author.name;
  // A gone operator's reply stays told apart from the sender's messages.
  return translate(
    operator,
    message.from === "operator"
      ? "feedback.deletedOperator"
      : "feedback.deleted",
  );
}

/**
 * Whether the reply form shows, and the note under it. A visitor is
 * reached only by mail, so without account mail there is no form.
 */
export function replyNote(
  entry: Pick<OperatorFeedback, "sender">,
  mail: boolean,
): { form: boolean; note: string | null } {
  const sender = entry.sender;
  switch (sender.type) {
    case "account":
      return {
        form: true,
        note: translate(
          operator,
          mail ? "feedback.note.accountMail" : "feedback.note.account",
          { name: sender.name },
        ),
      };
    case "visitor":
      return mail
        ? {
            form: true,
            note: translate(operator, "feedback.note.visitor", {
              email: sender.email,
            }),
          }
        : { form: false, note: translate(operator, "feedback.note.noMail") };
    case "deleted":
      // Nobody is left to read it; the API still keeps the reply.
      return { form: true, note: null };
  }
}

/** "Technische Angaben": label and value, `null` where nothing was sent. */
export function technicalDetails(
  context: OperatorFeedback["context"],
): { label: string; value: string | null; code?: boolean }[] {
  return [
    {
      label: translate(operator, "feedback.detail.app"),
      value: translate(operator, `feedback.client.${context.client}`),
    },
    {
      label: translate(operator, "feedback.detail.system"),
      value:
        context.os === null
          ? null
          : context.os === "other"
            ? translate(operator, "feedback.unknownSystem")
            : OS_NAMES[context.os],
    },
    {
      label: translate(operator, "feedback.detail.version"),
      value: context.app_version,
    },
    {
      label: translate(operator, "feedback.detail.language"),
      value: translate(language, `option.${context.locale}`),
    },
    {
      label: translate(operator, "feedback.detail.page"),
      value: context.page,
      code: true,
    },
  ];
}

/** An entry's "…" menu: only "Löschen…", which asks first. */
export function feedbackMenuEntries(on: { delete: () => void }): MenuEntry[] {
  return [
    {
      kind: "item",
      label: translate(operator, "feedback.menu.delete"),
      danger: true,
      onSelect: on.delete,
    },
  ];
}

/** Marks an entry done, or opens it again; `false` when it failed. */
export async function setFeedbackDone(
  entry: Pick<OperatorFeedback, "id">,
  done: boolean,
  refresh: () => void,
): Promise<boolean> {
  try {
    if (done) await remote.markFeedbackDone(entry.id);
    else await remote.reopenFeedback(entry.id);
    notify(
      translate(
        operator,
        done ? "toasts.feedbackDone" : "toasts.feedbackReopened",
      ),
    );
    return true;
  } catch (error) {
    notifyError(error, undefined, translate(operator, "toasts.feedbackGone"));
    return false;
  } finally {
    refresh();
  }
}

/** Asks, then deletes the entry with its image and thread. */
export function deleteFeedback(
  entry: Pick<OperatorFeedback, "id">,
  refresh: () => void,
): Promise<boolean> {
  return askThenRun({
    title: translate(operator, "dialogs.deleteFeedbackTitle"),
    message: translate(operator, "dialogs.deleteFeedbackMessage"),
    confirmLabel: translate(common, "delete"),
    run: () => remote.deleteFeedback(entry.id),
    done: translate(operator, "toasts.feedbackDeleted"),
    failed: "",
    gone: translate(operator, "toasts.feedbackGone"),
    refresh,
  });
}
