// One function per `/api/operator/*` route. No caching here; `queries.ts`
// owns that. Everyone but an operator gets `404 not_found` from these.

import { api } from "../api/client.ts";
import type {
  Account,
  AccountPage,
  AccountQuery,
  CreateInvitationInput,
  Invitation,
  InvitationPage,
  InvitationQuery,
  OperatorOverview,
  Report,
  ReportPage,
  ReportQuery,
  ResolveReportInput,
  ServerAdmin,
  ServerAdminPage,
  ServerQuery,
  Usage,
} from "./types.ts";

/** `?a=1&b=x` from the set values, or nothing when none is set. */
export function searchOf(
  params: Record<string, string | number | undefined>,
): string {
  const search = new URLSearchParams();
  for (const [name, value] of Object.entries(params)) {
    if (value === undefined || value === "") continue;
    search.set(name, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

export const getOverview = (signal?: AbortSignal) =>
  api<OperatorOverview>("/operator", { signal });

export const listAccounts = (query: AccountQuery, signal?: AbortSignal) =>
  api<AccountPage>(`/operator/accounts${searchOf(query)}`, { signal });

export const disableAccount = (id: string, reason?: string) =>
  api<Account>(`/operator/accounts/${id}/disable`, {
    method: "POST",
    body: reason ? { reason } : {},
  });

export const enableAccount = (id: string) =>
  api<Account>(`/operator/accounts/${id}/enable`, { method: "POST" });

export const listServerAdmins = (query: ServerQuery, signal?: AbortSignal) =>
  api<ServerAdminPage>(`/operator/servers${searchOf(query)}`, { signal });

export const delistServer = (id: string, reason?: string) =>
  api<ServerAdmin>(`/operator/servers/${id}/delist`, {
    method: "POST",
    body: reason ? { reason } : {},
  });

export const relistServer = (id: string) =>
  api<ServerAdmin>(`/operator/servers/${id}/relist`, { method: "POST" });

/** `reason` is required, 1 to 500 characters. */
export const blockServer = (id: string, reason: string) =>
  api<ServerAdmin>(`/operator/servers/${id}/block`, {
    method: "POST",
    body: { reason },
  });

export const unblockServer = (id: string) =>
  api<ServerAdmin>(`/operator/servers/${id}/unblock`, { method: "POST" });

export const listReports = (query: ReportQuery, signal?: AbortSignal) =>
  api<ReportPage>(`/operator/reports${searchOf(query)}`, { signal });

export const resolveReport = (id: string, input: ResolveReportInput) =>
  api<Report>(`/operator/reports/${id}/resolve`, {
    method: "POST",
    body: input,
  });

export const listInvitations = (query: InvitationQuery, signal?: AbortSignal) =>
  api<InvitationPage>(`/operator/invitations${searchOf(query)}`, { signal });

export const createInvitation = (input: CreateInvitationInput) =>
  api<Invitation>("/operator/invitations", { method: "POST", body: input });

export const revokeInvitation = (id: string) =>
  api<null>(`/operator/invitations/${id}`, { method: "DELETE" });

export const getUsage = (signal?: AbortSignal) =>
  api<Usage>("/operator/usage", { signal });
