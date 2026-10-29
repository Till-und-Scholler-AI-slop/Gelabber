// "Neue Nachricht": pick anyone from the servers you share and open the
// conversation with them, new or existing.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";

import { stampHolds, takeStamp, useUserId } from "../auth/scope.ts";
import {
  dmCandidates,
  filterCandidates,
  type DmCandidate,
} from "../dms/people.ts";
import { findCachedDm, useDms, useOpenDm } from "../dms/queries.ts";
import { useServerDetails, useServers } from "../servers/queries.ts";
import { Avatar } from "./Avatar.tsx";
import { Modal } from "./Modal.tsx";
import { Picker } from "./Picker.tsx";

export function NewMessageDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Modal open={open} onClose={onClose} title="Neue Nachricht">
      {open ? <NewMessageBody onClose={onClose} /> : null}
    </Modal>
  );
}

function NewMessageBody({ onClose }: { onClose: () => void }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const me = useUserId();
  const [query, setQuery] = useState("");
  const { data: servers, isPending } = useServers();
  const details = useServerDetails((servers ?? []).map((server) => server.id));
  const { data: dms } = useDms();
  const openDm = useOpenDm();
  // A pick still on its way when the dialog closed must not navigate.
  const request = useRef(0);
  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );
  const people = useMemo(
    () => dmCandidates(details, dms ?? [], me),
    [details, dms, me],
  );
  const loading = isPending || details.length < (servers?.length ?? 0);

  const write = (person: DmCandidate) => {
    const stamp = takeStamp();
    const current = ++request.current;
    const go = (id: string) => {
      if (!stampHolds(stamp) || current !== request.current) return;
      onClose();
      void navigate({ to: "/d/$channelId", params: { channelId: id } });
    };
    const cached = person.dmId ?? findCachedDm(client, me, person.id)?.id;
    if (cached) go(cached);
    else openDm.mutate(person.id, { onSuccess: (dm) => go(dm.id) });
  };

  return (
    <Picker
      label="Person"
      placeholder="Name suchen"
      query={query}
      onQuery={setQuery}
      busy={openDm.isPending}
      options={filterCandidates(people, query).map((person) => ({
        id: person.id,
        value: person,
        label: person.name,
        detail: person.dmId
          ? "Unterhaltung fortsetzen"
          : person.servers.join(", "),
        icon: <Avatar name={person.name} url={person.avatar_url} decorative />,
      }))}
      onPick={(option) => write(option.value)}
      empty={
        loading
          ? "Wird geladen…"
          : people.length === 0
            ? "Keine Personen. Direktnachrichten gehen an Mitglieder deiner Server."
            : `Niemand gefunden für „${query.trim()}“.`
      }
    />
  );
}
