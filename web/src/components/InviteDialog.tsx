// "Einladen": opens straight onto a working link, one click from the
// clipboard or the share sheet. Any member may invite; expiry and use limits
// sit behind a disclosure.

import { useRouterState } from "@tanstack/react-router";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { ApiError } from "../api/client.ts";
import { useUserId } from "../auth/scope.ts";
import { errorMessage } from "../auth/rules.ts";
import { inviteUrl } from "../servers/api.ts";
import { canShare, copyText, share } from "../servers/clipboard.ts";
import {
  dismissInvitePrompt,
  useInvitePrompt,
} from "../servers/invitePrompt.ts";
import {
  DEFAULT_INVITE_TERMS,
  EXPIRY_OPTIONS,
  USES_OPTIONS,
  inviteInput,
  termsSummary,
  type InviteTerms,
} from "../servers/inviteTerms.ts";
import { useCreateInvite, useRevokeInvite } from "../servers/queries.ts";
import type { Invite } from "../servers/types.ts";
import { ChevronIcon, ShareIcon } from "./Icons.tsx";
import { inviteSession, type InviteSession } from "./inviteSession.ts";
import { LoadError } from "./LoadError.tsx";
import { Modal } from "./Modal.tsx";
import { Button } from "./ui/Button.tsx";
import { ChipGroup } from "./ui/ChipGroup.tsx";
import { CopyButton } from "./ui/CopyButton.tsx";
import { InlineStatus } from "./ui/InlineStatus.tsx";
import { useInlineStatus } from "./ui/useInlineStatus.ts";
import "./invite.css";

type InviteServer = { id: string; name: string };

export function InviteDialog({
  server,
  open,
  onClose,
}: {
  server: InviteServer;
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Modal open={open} onClose={onClose} title={`Zu ${server.name} einladen`}>
      {open ? <InviteLinkPanel server={server} /> : null}
    </Modal>
  );
}

/**
 * The invite dialog a new server opens with (`promptInvite`). It waits until
 * the server's page has rendered: a drawer that page opens after the dialog
 * would take Esc away from it (Chrome).
 */
export function InvitePromptHost() {
  const userId = useUserId();
  const prompt = useInvitePrompt((state) => state.prompt);
  const id = prompt && prompt.userId === userId ? prompt.id : null;
  const landed = useRouterState({
    select: (state) =>
      id !== null &&
      state.status === "idle" &&
      (state.location.pathname === `/s/${id}` ||
        state.location.pathname.startsWith(`/s/${id}/`)),
  });
  const [shown, setShown] = useState<string | null>(null);
  useEffect(() => {
    if (!landed || id === null) return;
    const frame = requestAnimationFrame(() => setShown(id));
    return () => cancelAnimationFrame(frame);
  }, [landed, id]);
  if (!prompt || id === null || shown !== id) return null;
  const close = () => {
    dismissInvitePrompt();
    // What created the server is gone; carry on in the server's page.
    requestAnimationFrame(() => {
      if (document.activeElement === document.body)
        document
          .getElementById("workspace-content")
          ?.focus({ preventScroll: true });
    });
  };
  return <InviteDialog key={prompt.id} server={prompt} open onClose={close} />;
}

/**
 * Creates a link as it mounts and shows it ready to copy. Changing the
 * terms makes a new link; the previous one is revoked unless it was
 * already copied or shared, since someone may be holding it. Closing
 * revokes a link nobody took (inviteSession.ts).
 */
export function InviteLinkPanel({ server }: { server: InviteServer }) {
  const create = useCreateInvite(server.id);
  const revoke = useRevokeInvite(server.id);
  // `terms` is what the chips ask for; `current` the link on screen and the
  // terms it was made with, which differ while a new link is on its way.
  const [terms, setTerms] = useState<InviteTerms>(DEFAULT_INVITE_TERMS);
  const [current, setCurrent] = useState<{
    invite: Invite;
    terms: InviteTerms;
  } | null>(null);
  const invite = current?.invite ?? null;
  const started = useRef(false);
  const { status, show } = useInlineStatus();
  const mutations = useRef({
    create: create.mutateAsync,
    revoke: revoke.mutate,
  });
  useLayoutEffect(() => {
    mutations.current = { create: create.mutateAsync, revoke: revoke.mutate };
  });
  const session = useRef<InviteSession | null>(null);

  // One link per opening; the ref also covers StrictMode's second mount.
  useEffect(() => {
    session.current ??= inviteSession({
      create: (next) => mutations.current.create(inviteInput(next)),
      revoke: (code) => mutations.current.revoke(code),
      shown: (created, next, replaced) => {
        setCurrent({ invite: created, terms: next });
        if (replaced)
          show({
            tone: "neutral",
            message: `Neuer Link · ${termsSummary(next)}`,
          });
      },
      // Shown from `create.error`; the chips go back to the link shown.
      failed: (shownTerms) => {
        if (shownTerms) setTerms(shownTerms);
      },
    });
    const links = session.current;
    links.open();
    if (!started.current) {
      started.current = true;
      links.request(DEFAULT_INVITE_TERMS, false);
    }
    return () => links.close();
  }, [show]);

  const changeTerms = (next: InviteTerms) => {
    setTerms(next);
    session.current?.request(next, true);
  };
  // A link for the chips' terms is still to come.
  const making =
    create.isPending ||
    (current !== null &&
      (current.terms.expiry !== terms.expiry ||
        current.terms.uses !== terms.uses));

  const link = invite ? inviteUrl(invite.code) : "";
  const summary = termsSummary(current?.terms ?? terms);

  const copy = async (): Promise<boolean> => {
    if (making) {
      show({ tone: "neutral", message: "Link wird erstellt…" });
      return false;
    }
    if (!link) return false;
    if (await copyText(link)) {
      session.current?.handOut();
      show({ tone: "success", message: `Link kopiert · ${summary}` });
      return true;
    }
    show({
      tone: "error",
      message: "Kopieren fehlgeschlagen. Kopiere den Link manuell.",
    });
    return false;
  };

  if (!invite && create.isError) {
    return (
      <LoadError
        message={`Der Einladungslink konnte nicht erstellt werden. ${errorMessage(
          create.error instanceof ApiError ? create.error.code : "internal",
        )}`}
        pending={create.isPending}
        onRetry={() => session.current?.request(terms, false)}
      />
    );
  }

  return (
    <div className="invite-panel">
      <div className="invite-panel__link">
        <input
          readOnly
          value={making ? "Link wird erstellt…" : link}
          aria-label="Einladungslink"
          aria-busy={making || undefined}
          onFocus={(event) => event.currentTarget.select()}
          onCopy={() => session.current?.handOut()}
          className="ui-field invite-panel__url"
        />
        {/* The status line below says it, with the link's terms. */}
        <CopyButton
          variant="primary"
          onCopy={copy}
          announce={false}
          initialFocus
        >
          Link kopieren
        </CopyButton>
      </div>
      <div className="invite-panel__status">
        <InlineStatus status={status} />
        {status ? null : <span className="invite-panel__terms">{summary}</span>}
      </div>
      {canShare() ? (
        <Button
          icon={<ShareIcon size={16} />}
          onClick={() =>
            link &&
            !making &&
            void share({
              title: `Einladung zu ${server.name}`,
              text: `Tritt ${server.name} auf Gelabber bei:`,
              url: link,
            }).then((shared) => {
              if (shared) session.current?.handOut();
            })
          }
        >
          Teilen…
        </Button>
      ) : null}
      {create.isError && invite ? (
        <p role="alert" className="ui-field-error">
          Der neue Link konnte nicht erstellt werden. Der bisherige gilt weiter.
        </p>
      ) : null}
      <details className="invite-panel__terms-editor">
        <summary>
          Ablauf &amp; Limit ändern
          <ChevronIcon size={14} />
        </summary>
        <div className="invite-panel__chips">
          <span className="invite-panel__caps">Läuft ab nach</span>
          <ChipGroup
            label="Läuft ab nach"
            options={EXPIRY_OPTIONS}
            value={terms.expiry}
            onChange={(expiry) => changeTerms({ ...terms, expiry })}
          />
          <span className="invite-panel__caps">Wie oft nutzbar</span>
          <ChipGroup
            label="Wie oft nutzbar"
            options={USES_OPTIONS}
            value={terms.uses}
            onChange={(uses) => changeTerms({ ...terms, uses })}
          />
        </div>
      </details>
    </div>
  );
}
