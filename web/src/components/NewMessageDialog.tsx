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
import { useT } from "../i18n/catalog.ts";
import { dms as dmsCatalog } from "../i18n/catalogs/dms.ts";
import { common } from "../i18n/catalogs/common.ts";

export function NewMessageDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const t = useT(dmsCatalog);
  return (
    <Modal open={open} onClose={onClose} title={t("newMessage")}>
      {open ? <NewMessageBody onClose={onClose} /> : null}
    </Modal>
  );
}

function NewMessageBody({ onClose }: { onClose: () => void }) {
  const t = useT(dmsCatalog);
  const tc = useT(common);
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
      label={t("newMessage.person")}
      placeholder={t("newMessage.placeholder")}
      query={query}
      onQuery={setQuery}
      busy={openDm.isPending}
      options={filterCandidates(people, query).map((person) => ({
        id: person.id,
        value: person,
        label: person.name,
        detail: person.dmId
          ? t("newMessage.continue")
          : person.servers.join(", "),
        icon: <Avatar name={person.name} url={person.avatar_url} decorative />,
      }))}
      onPick={(option) => write(option.value)}
      empty={
        loading
          ? tc("loading")
          : people.length === 0
            ? t("newMessage.nobody")
            : t("newMessage.notFound", { query: query.trim() })
      }
    />
  );
}
