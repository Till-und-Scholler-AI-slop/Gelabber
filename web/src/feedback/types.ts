// Feedback to the instance's operators (v0.10): what `/api/feedback…`
// answers a signed-in person.

import type { DeviceOs } from "../auth/devices.ts";
import type { Locale } from "../i18n/locale.ts";

export type FeedbackKind = "idea" | "problem" | "praise";

export const FEEDBACK_KINDS: readonly FeedbackKind[] = [
  "idea",
  "problem",
  "praise",
];

/** `GET /api/feedback/status`. Off, `unread` is 0. */
export type FeedbackStatus = {
  enabled: boolean;
  /** The caller's entries with an operator reply they have not seen. */
  unread: number;
};

/** What a client sends along with an entry, and nothing else. */
export type FeedbackContext = {
  client: "web" | "desktop";
  os: DeviceOs;
  app_version: string;
  locale: Locale;
  /** The path, without query and fragment, at most 300 characters. */
  page: string;
};

/** The context as an entry carries it back. */
export type EntryContext = {
  client: "web" | "desktop" | "form";
  os: DeviceOs | null;
  app_version: string | null;
  locale: Locale;
  page: string | null;
};

export type FeedbackImage = {
  url: string;
  content_type: string;
  size: number;
};

/** One message of a thread; operators stay unnamed. */
export type FeedbackMessage = {
  id: string;
  from: "you" | "operators";
  text: string;
  created_at: string;
  /** An operator reply the person had not seen when the list was read. */
  new: boolean;
};

/** An entry as its sender sees it. */
export type FeedbackEntry = {
  id: string;
  kind: FeedbackKind;
  status: "open" | "done";
  text: string;
  created_at: string;
  last_activity_at: string;
  image: FeedbackImage | null;
  context: EntryContext;
  unread: boolean;
  messages: FeedbackMessage[];
};

/** `GET /api/feedback?offset=`, 20 entries a page. */
export type FeedbackPage = {
  entries: FeedbackEntry[];
  total: number;
};

/** `POST /api/feedback`. `image` is the id of a staged upload. */
export type NewFeedback = {
  kind: FeedbackKind;
  text: string;
  image?: string;
  context: FeedbackContext;
};
