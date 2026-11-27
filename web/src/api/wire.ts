// What the API and the gateway hand over, checked against the types before
// the app keeps it (GEL-420). Every supported server sends these objects
// whole; one that lacks a field the types promise is a server bug, a cut
// answer or a broken frame. It is not patched with defaults: a row of a list
// is dropped and reported once, and a single object turns the request into
// an error. Otherwise one bad row reaches the renderer and takes the view
// with it.

import type { Message, MessagePage } from "../messages/types.ts";
import type { ReadState } from "../messages/readState.ts";
import type { ReadSync } from "../ws/protocol.ts";
import type {
  Category,
  Channel,
  Member,
  Server,
  ServerDetail,
  ServerRole,
} from "../servers/types.ts";
import { ApiError } from "./client.ts";
import {
  absentOr,
  count,
  countOrNull,
  flag,
  listOf,
  record,
  reportMalformed,
  text,
  textOrNull,
  texts,
  type Check,
  type Fields,
} from "./shapes.ts";

export { isUser, reportMalformed, resetMalformedReports } from "./shapes.ts";

function isAttachment(value: unknown): boolean {
  return (
    record(value) &&
    text(value.id) &&
    text(value.filename) &&
    text(value.content_type) &&
    count(value.size)
  );
}

function isReaction(value: unknown): boolean {
  return record(value) && text(value.emoji) && texts(value.user_ids);
}

export const isMessage: Check<Message> = (value): value is Message =>
  record(value) &&
  text(value.id) &&
  text(value.channel_id) &&
  text(value.content) &&
  text(value.created_at) &&
  textOrNull(value.edited_at) &&
  record(value.author) &&
  text(value.author.id) &&
  text(value.author.name) &&
  textOrNull(value.author.avatar_url) &&
  listOf(value.attachments, isAttachment) &&
  listOf(value.reactions, isReaction) &&
  texts(value.mentions) &&
  flag(value.mention_everyone) &&
  absentOr(value.revision, count) &&
  absentOr(value.created_order, count);

export const isServer: Check<Server> = (value): value is Server =>
  record(value) &&
  text(value.id) &&
  text(value.name) &&
  text(value.owner_id) &&
  absentOr(value.icon_url, textOrNull) &&
  text(value.created_at) &&
  text(value.role) &&
  texts(value.permissions) &&
  texts(value.member_permissions) &&
  flag(value.blocked);

const isCategory: Check<Category> = (value): value is Category =>
  record(value) &&
  text(value.id) &&
  text(value.server_id) &&
  text(value.name) &&
  text(value.created_at);

export const isChannel: Check<Channel> = (value): value is Channel =>
  record(value) &&
  text(value.id) &&
  text(value.server_id) &&
  textOrNull(value.category_id) &&
  text(value.name) &&
  text(value.kind) &&
  text(value.created_at) &&
  flag(value.limited) &&
  texts(value.role_ids);

export const isMember: Check<Member> = (value): value is Member =>
  record(value) &&
  text(value.user_id) &&
  text(value.name) &&
  textOrNull(value.avatar_url) &&
  text(value.joined_at) &&
  text(value.role) &&
  texts(value.role_ids) &&
  absentOr(value.status_text, textOrNull) &&
  absentOr(value.accent_color, textOrNull) &&
  absentOr(value.avatar_decoration, textOrNull);

const isRole: Check<ServerRole> = (value): value is ServerRole =>
  record(value) &&
  text(value.id) &&
  text(value.server_id) &&
  text(value.name) &&
  text(value.color) &&
  texts(value.permissions) &&
  count(value.position) &&
  text(value.created_at);

export const isServerDetail: Check<ServerDetail> = (
  value,
): value is ServerDetail =>
  isServer(value) &&
  listOf((value as Fields).categories, isCategory) &&
  listOf((value as Fields).channels, isChannel) &&
  listOf((value as Fields).members, isMember) &&
  listOf((value as Fields).roles, isRole);

export const isReadSync: Check<ReadSync> = (value): value is ReadSync =>
  record(value) &&
  text(value.channel_id) &&
  textOrNull(value.server_id) &&
  textOrNull(value.read_message_id) &&
  textOrNull(value.read_at) &&
  countOrNull(value.read_order) &&
  text(value.membership_at) &&
  countOrNull(value.latest_order) &&
  count(value.unread_count) &&
  count(value.mention_count);

export const isReadState: Check<ReadState> = (value): value is ReadState =>
  isReadSync(value) &&
  absentOr((value as Fields).level, text) &&
  absentOr((value as Fields).muted, flag) &&
  absentOr((value as Fields).muted_until, textOrNull);

/** A page of messages with its whole rows; the others are reported and left
 *  out. */
export function wholePage(value: unknown): MessagePage {
  if (!record(value) || !flag(value.has_more))
    throw malformed("message page", value);
  const page: MessagePage = {
    ...(value as MessagePage),
    messages: wholeRows(value.messages, isMessage, "message"),
  };
  // Paging goes on from the oldest row the server sent, also when that row,
  // or every row, was left out (`encodeCursor` in messages/pages.ts).
  const oldest: unknown = (value.messages as unknown[])[0];
  if (page.older === undefined && record(oldest)) {
    if (text(oldest.created_at) && text(oldest.id))
      page.older = `${oldest.created_at}|${oldest.id}`;
  }
  return page;
}

/** The rows of a list that are whole; the others are reported and left out.
 *  Something that is no list at all fails the request. */
export function wholeRows<T>(
  value: unknown,
  check: Check<T>,
  what: string,
): T[] {
  if (!Array.isArray(value)) throw malformed(`${what} list`, value);
  return value.filter((row): row is T => {
    if (check(row)) return true;
    reportMalformed(what, row);
    return false;
  });
}

/** A single object, whole, or the request fails. */
export function whole<T>(value: unknown, check: Check<T>, what: string): T {
  if (check(value)) return value;
  throw malformed(what, value);
}

/** The error of a request whose answer is not what the server sends; says
 *  so once in the console. */
export function malformed(what: string, value: unknown): ApiError {
  reportMalformed(what, value);
  return new ApiError(
    "malformed_answer",
    0,
    `The server sent a malformed ${what}.`,
  );
}

/** A request whose answer was rejected: it may still have done its work. */
export function isMalformedAnswer(error: unknown): boolean {
  return error instanceof ApiError && error.code === "malformed_answer";
}
