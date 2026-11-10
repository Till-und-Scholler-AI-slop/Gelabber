// What an operator can do to a server, from the Server page and from a
// report: write to its owner, take it out of the directory or allow it
// again, block it or lift the block. Each change asks first; blocking asks
// for a reason. The menus are data, so tests can choose their entries.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";

import { stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/store.ts";
import { confirmAction } from "../components/confirm.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { notify, notifyError } from "../components/toasts.ts";
import { findCachedDm, useOpenDm } from "../dms/queries.ts";
import { translate } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import * as remote from "./api.ts";
import { ReasonDialog, type ReasonRequest } from "./parts.tsx";
import { useOperatorRefresh } from "./queries.ts";
import type { DirectoryState, Report } from "./types.ts";

export type ServerTarget = {
  id: string;
  name: string;
  directory: DirectoryState;
  blocked: boolean;
};

export type ServerHandlers = {
  /** Absent when there is nobody to write to (the owner is you). */
  message?: () => void;
  delist: () => void;
  relist: () => void;
  block: () => void;
  unblock: () => void;
};

/** A row's menu on the Server page. */
export function serverMenuEntries(
  server: Pick<ServerTarget, "directory" | "blocked">,
  on: ServerHandlers,
): MenuEntry[] {
  const entries: MenuEntry[] = [];
  if (on.message)
    entries.push({
      kind: "item",
      label: translate(operator, "servers.menu.message"),
      onSelect: on.message,
    });
  if (entries.length > 0) entries.push({ kind: "separator" });
  entries.push(
    server.directory === "delisted"
      ? {
          kind: "item",
          label: translate(operator, "servers.menu.relist"),
          onSelect: on.relist,
        }
      : {
          kind: "item",
          label: translate(operator, "servers.menu.delist"),
          onSelect: on.delist,
        },
    server.blocked
      ? {
          kind: "item",
          label: translate(operator, "servers.menu.unblock"),
          onSelect: on.unblock,
        }
      : {
          kind: "item",
          label: translate(operator, "servers.menu.block"),
          danger: true,
          onSelect: on.block,
        },
  );
  return entries;
}

export type ReportHandlers = {
  open: () => void;
  dismiss: () => void;
  delist: () => void;
  block: () => void;
};

/**
 * A report's menu. An open one can be closed three ways; a closed one only
 * leads to its server.
 */
export function reportMenuEntries(
  report: Pick<Report, "status" | "server">,
  on: ReportHandlers,
): MenuEntry[] {
  const open: MenuEntry = {
    kind: "item",
    label: translate(operator, "reports.menu.open"),
    onSelect: on.open,
  };
  if (report.status !== "open") return [open];
  const entries: MenuEntry[] = [
    open,
    { kind: "separator" },
    {
      kind: "item",
      label: translate(operator, "reports.menu.dismiss"),
      onSelect: on.dismiss,
    },
    {
      kind: "item",
      label: translate(operator, "servers.menu.delist"),
      onSelect: on.delist,
    },
  ];
  if (!report.server.blocked)
    entries.push({
      kind: "item",
      label: translate(operator, "servers.menu.block"),
      danger: true,
      onSelect: on.block,
    });
  return entries;
}

/**
 * Asks, then calls the route, says so, and refetches the area. `false` when
 * the operator said no or the call failed (a toast says which).
 */
async function askThenRun({
  title,
  message,
  confirmLabel,
  tone,
  run,
  done,
  failed,
  refresh,
}: {
  title: string;
  message: string;
  confirmLabel: string;
  tone?: "danger" | "neutral";
  run: () => Promise<unknown>;
  done: string;
  failed: string;
  refresh: () => void;
}): Promise<boolean> {
  if (!(await confirmAction({ title, message, confirmLabel, tone })))
    return false;
  try {
    await run();
    notify(done);
    return true;
  } catch (error) {
    notifyError(error, failed, translate(operator, "toasts.serverGone"));
    return false;
  } finally {
    refresh();
  }
}

export function delistServer(server: ServerTarget, refresh: () => void) {
  const name = server.name;
  return askThenRun({
    title: translate(operator, "dialogs.delistTitle", { name }),
    message: translate(operator, "dialogs.delistMessage"),
    confirmLabel: translate(operator, "servers.menu.delist"),
    run: () => remote.delistServer(server.id),
    done: translate(operator, "toasts.delisted", { name }),
    failed: translate(operator, "toasts.delistFailed"),
    refresh,
  });
}

export function relistServer(server: ServerTarget, refresh: () => void) {
  const name = server.name;
  return askThenRun({
    title: translate(operator, "dialogs.relistTitle", { name }),
    message: translate(operator, "dialogs.relistMessage"),
    confirmLabel: translate(operator, "servers.menu.relist"),
    tone: "neutral",
    run: () => remote.relistServer(server.id),
    done: translate(operator, "toasts.relisted", { name }),
    failed: translate(operator, "toasts.relistFailed"),
    refresh,
  });
}

export function unblockServer(server: ServerTarget, refresh: () => void) {
  const name = server.name;
  return askThenRun({
    title: translate(operator, "dialogs.unblockTitle", { name }),
    message: translate(operator, "dialogs.unblockMessage"),
    confirmLabel: translate(operator, "servers.menu.unblock"),
    tone: "neutral",
    run: () => remote.unblockServer(server.id),
    done: translate(operator, "toasts.unblocked", { name }),
    failed: translate(operator, "toasts.unblockFailed"),
    refresh,
  });
}

/** The dialog that blocks a server: a reason is required. */
export function blockRequest(
  server: ServerTarget,
  refresh: () => void,
): ReasonRequest {
  const name = server.name;
  return {
    title: translate(operator, "dialogs.blockTitle", { name }),
    message: translate(operator, "dialogs.blockMessage"),
    label: translate(operator, "dialogs.blockReason"),
    field: "reason",
    required: true,
    confirmLabel: translate(operator, "servers.menu.block"),
    danger: true,
    submit: async (reason) => {
      try {
        await remote.blockServer(server.id, reason);
      } finally {
        refresh();
      }
      notify(translate(operator, "toasts.blocked", { name }));
    },
  };
}

/** The dialog that closes a report without action: a note is optional. */
export function dismissRequest(
  report: Pick<Report, "id" | "server">,
  refresh: () => void,
): ReasonRequest {
  return {
    title: translate(operator, "dialogs.dismissTitle", {
      name: report.server.name,
    }),
    label: translate(operator, "dialogs.dismissNote"),
    field: "note",
    required: false,
    confirmLabel: translate(operator, "reports.menu.dismiss"),
    submit: async (note) => {
      try {
        await remote.resolveReport(report.id, {
          status: "dismissed",
          ...(note ? { note } : {}),
        });
      } finally {
        refresh();
      }
      notify(translate(operator, "toasts.dismissed"));
    },
  };
}

/** Opens (or finds) the conversation with someone and goes there. */
function useMessagePerson() {
  const me = useSession((state) => state.user?.id);
  const client = useQueryClient();
  const navigate = useNavigate();
  const openDm = useOpenDm();
  return (userId: string) => {
    if (!me || userId === me) return;
    const stamp = takeStamp();
    const go = (id: string) => {
      if (!stampHolds(stamp)) return;
      void navigate({ to: "/d/$channelId", params: { channelId: id } });
    };
    const cached = findCachedDm(client, me, userId);
    if (cached) go(cached.id);
    else openDm.mutate(userId, { onSuccess: (dm) => go(dm.id) });
  };
}

/**
 * The server actions of a page plus the reason dialog they open; the page
 * renders `dialog` once. `ask` opens any reason dialog (a report's note).
 */
export function useServerActions() {
  const me = useSession((state) => state.user?.id);
  const refresh = useOperatorRefresh();
  const message = useMessagePerson();
  const [request, setRequest] = useState<ReasonRequest | null>(null);
  const handlers = (
    server: ServerTarget,
    ownerId?: string,
  ): ServerHandlers => ({
    message:
      ownerId !== undefined && ownerId !== me
        ? () => message(ownerId)
        : undefined,
    delist: () => void delistServer(server, refresh),
    relist: () => void relistServer(server, refresh),
    block: () => setRequest(blockRequest(server, refresh)),
    unblock: () => void unblockServer(server, refresh),
  });
  return {
    handlers,
    refresh,
    ask: setRequest,
    dialog: <ReasonDialog request={request} onClose={() => setRequest(null)} />,
  };
}
