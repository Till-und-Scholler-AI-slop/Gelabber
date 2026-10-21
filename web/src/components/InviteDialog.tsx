// "Leute einladen": opens straight onto a working link, one click from the
// clipboard or the share sheet. Any member may invite; expiry and use limits
// sit behind a disclosure.

import { useEffect, useRef, useState } from "react";

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
import { ChevronIcon, CopyIcon, ShareIcon } from "./Icons.tsx";
import { LoadError } from "./LoadError.tsx";
import { Modal } from "./Modal.tsx";
import { Button } from "./ui/Button.tsx";
import { ChipGroup } from "./ui/ChipGroup.tsx";
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
    <Modal
      open={open}
      onClose={onClose}
      title={`Leute zu ${server.name} einladen`}
    >
      {open ? <InviteLinkPanel server={server} /> : null}
    </Modal>
  );
}

/** The invite dialog a new server opens with (`promptInvite`). */
export function InvitePromptHost() {
  const userId = useUserId();
  const prompt = useInvitePrompt((state) => state.prompt);
  if (!prompt || prompt.userId !== userId) return null;
  return (
    <InviteDialog
      key={prompt.id}
      server={prompt}
      open
      onClose={dismissInvitePrompt}
    />
  );
}

/**
 * Creates a link as it mounts and shows it ready to copy. Changing the
 * terms makes a new link; the previous one is revoked unless it was
 * already copied or shared, since someone may be holding it.
 */
export function InviteLinkPanel({ server }: { server: InviteServer }) {
  const create = useCreateInvite(server.id);
  const revoke = useRevokeInvite(server.id);
  const [terms, setTerms] = useState<InviteTerms>(DEFAULT_INVITE_TERMS);
  const [invite, setInvite] = useState<Invite | null>(null);
  const handedOut = useRef(false);
  const started = useRef(false);
  const latest = useRef(0);
  const { status, show } = useInlineStatus();

  const makeLink = (next: InviteTerms, previous: Invite | null) => {
    const request = ++latest.current;
    create
      .mutateAsync(inviteInput(next))
      .then((created) => {
        // Terms changed again while this one was made: nobody saw it.
        if (request !== latest.current) {
          revoke.mutate(created.code);
          return;
        }
        setInvite(created);
        if (previous) {
          if (!handedOut.current) revoke.mutate(previous.code);
          show({
            tone: "neutral",
            message: `Neuer Link · ${termsSummary(next)}`,
          });
        }
        handedOut.current = false;
      })
      .catch(() => {
        // Shown from `create.error`.
      });
  };

  // One link per opening; the ref also covers StrictMode's second mount.
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    makeLink(DEFAULT_INVITE_TERMS, null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const changeTerms = (next: InviteTerms) => {
    setTerms(next);
    makeLink(next, invite);
  };

  const link = invite ? inviteUrl(invite.code) : "";
  const summary = termsSummary(terms);

  const copy = async () => {
    if (!link) return;
    if (await copyText(link)) {
      handedOut.current = true;
      show({ tone: "success", message: `Link kopiert · ${summary}` });
    } else {
      show({
        tone: "error",
        message: "Kopieren ging nicht – markiere den Link und kopiere ihn.",
      });
    }
  };

  if (!invite && create.isError) {
    return (
      <LoadError
        message={`Der Einladungslink konnte nicht erstellt werden. ${errorMessage(
          create.error instanceof ApiError ? create.error.code : "internal",
        )}`}
        pending={create.isPending}
        onRetry={() => makeLink(terms, null)}
      />
    );
  }

  return (
    <div className="invite-panel">
      <p className="invite-panel__lead">
        Schick den Link an deine Leute. Wer ihn öffnet, legt ein Konto an oder
        meldet sich an und landet direkt in {server.name}.
      </p>
      <div className="invite-panel__link">
        <input
          readOnly
          value={create.isPending ? "Link wird erstellt…" : link}
          aria-label="Einladungslink"
          aria-busy={create.isPending || undefined}
          onFocus={(event) => event.currentTarget.select()}
          onCopy={() => {
            handedOut.current = true;
          }}
          className="ui-field invite-panel__url"
        />
        <Button
          variant="primary"
          icon={<CopyIcon size={16} />}
          onClick={() => void copy()}
          initialFocus
        >
          Link kopieren
        </Button>
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
            void share({
              title: `Einladung zu ${server.name}`,
              text: `Komm zu ${server.name} auf Gelabber:`,
              url: link,
            }).then((shared) => {
              if (shared) handedOut.current = true;
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
