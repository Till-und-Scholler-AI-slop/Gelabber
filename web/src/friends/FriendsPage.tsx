// `/d/friends` (#563): the friend list (all · pending), adding a friend by
// code, and the signed-in user's own code.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useId, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { fieldMessages } from "../auth/rules.ts";
import { stampHolds, takeStamp, useUserId } from "../auth/scope.ts";
import { confirmAction } from "../components/confirm.ts";
import { LoadError } from "../components/LoadError.tsx";
import { PresenceAvatar } from "../components/PresenceAvatar.tsx";
import { notify } from "../components/toasts.ts";
import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { FormField } from "../components/ui/FormField.tsx";
import { Input } from "../components/ui/Input.tsx";
import { MainSkeleton } from "../components/ui/Skeleton.tsx";
import { findCachedDm, useOpenDm } from "../dms/queries.ts";
import { useT } from "../i18n/catalog.ts";
import { friends as friendsCatalog } from "../i18n/catalogs/friends.ts";
import { FriendCodeCard } from "./FriendCodeCard.tsx";
import {
  useAcceptFriend,
  useDeclineFriend,
  useFriends,
  useRemoveFriend,
  useSendFriendRequest,
  useWithdrawFriend,
} from "./queries.ts";
import type { Friend, FriendRequest } from "./types.ts";
import "./friends.css";

type Tab = "all" | "pending" | "add";

export function FriendsPage() {
  const t = useT(friendsCatalog);
  const { data, isPending, isError, isFetching, refetch } = useFriends({
    live: true,
  });
  const pendingCount = data ? data.incoming.length : 0;
  const [tab, setTab] = useState<Tab>("all");

  return (
    <div className="friends-page">
      <header className="friends-page__header">
        <h1>{t("page.title")}</h1>
        <ChipGroup<Tab>
          label={t("tabs.label")}
          value={tab}
          onChange={setTab}
          options={[
            { value: "all", label: t("tab.all") },
            {
              value: "pending",
              label:
                pendingCount > 0
                  ? `${t("tab.pending")} (${pendingCount})`
                  : t("tab.pending"),
            },
            { value: "add", label: t("tab.add") },
          ]}
        />
      </header>
      <div className="friends-page__body">
        {tab === "add" ? (
          <>
            <AddFriend />
            <FriendCodeCard />
          </>
        ) : isPending && !isError ? (
          <MainSkeleton kind="list" />
        ) : isError || !data ? (
          <LoadError
            message={t("list.loadFailed")}
            pending={isFetching}
            onRetry={() => void refetch()}
          />
        ) : tab === "all" ? (
          <FriendRows friends={data.friends} />
        ) : (
          <PendingRows incoming={data.incoming} outgoing={data.outgoing} />
        )}
      </div>
    </div>
  );
}

function FriendRows({ friends }: { friends: Friend[] }) {
  const t = useT(friendsCatalog);
  if (friends.length === 0)
    return <p className="friends-empty">{t("list.empty")}</p>;
  return (
    <ul className="friend-list" aria-label={t("tab.all")}>
      {friends.map((friend) => (
        <FriendRow key={friend.user.id} friend={friend} />
      ))}
    </ul>
  );
}

function FriendRow({ friend }: { friend: Friend }) {
  const t = useT(friendsCatalog);
  const client = useQueryClient();
  const navigate = useNavigate();
  const me = useUserId();
  const openDm = useOpenDm();
  const remove = useRemoveFriend();
  const { user } = friend;
  const status = friend.status ?? "x";
  const message = () => {
    const stamp = takeStamp();
    const go = (id: string) => {
      if (stampHolds(stamp))
        void navigate({ to: "/d/$channelId", params: { channelId: id } });
    };
    const cached = findCachedDm(client, me, user.id);
    if (cached) go(cached.id);
    else openDm.mutate(user.id, { onSuccess: (dm) => go(dm.id) });
  };
  const unfriend = async () => {
    const ok = await confirmAction({
      title: t("action.removeConfirm", { name: user.name }),
      message: t("action.removeHint"),
      confirmLabel: t("action.remove"),
      tone: "danger",
    });
    if (ok) remove.mutate(user.id);
  };
  return (
    <li className="friend-row">
      <PresenceAvatar name={user.name} url={user.avatar_url} status={status} />
      <span className="friend-row__name">
        <span>{user.name}</span>
        {friend.status ? (
          <span className="friend-row__detail">
            {friend.status === "o" ? t("status.online") : t("status.idle")}
          </span>
        ) : null}
      </span>
      <span className="friend-row__actions">
        <Button size="sm" disabled={openDm.isPending} onClick={message}>
          {t("action.message")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={remove.isPending}
          onClick={() => void unfriend()}
        >
          {t("action.remove")}
        </Button>
      </span>
    </li>
  );
}

function PendingRows({
  incoming,
  outgoing,
}: {
  incoming: FriendRequest[];
  outgoing: FriendRequest[];
}) {
  const t = useT(friendsCatalog);
  const accept = useAcceptFriend();
  const decline = useDeclineFriend();
  const withdraw = useWithdrawFriend();
  if (incoming.length === 0 && outgoing.length === 0)
    return <p className="friends-empty">{t("pending.empty")}</p>;
  return (
    <>
      {incoming.length > 0 ? (
        <section aria-labelledby="friends-incoming">
          <h2 id="friends-incoming" className="friends-section-title">
            {t("pending.incoming")}
          </h2>
          <ul className="friend-list">
            {incoming.map((request) => (
              <li key={request.user.id} className="friend-row">
                <PresenceAvatar
                  name={request.user.name}
                  url={request.user.avatar_url}
                  status="x"
                />
                <span className="friend-row__name">{request.user.name}</span>
                <span className="friend-row__actions">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={accept.isPending}
                    onClick={() => accept.mutate(request.user.id)}
                  >
                    {t("action.accept")}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={decline.isPending}
                    onClick={() => decline.mutate(request.user.id)}
                  >
                    {t("action.decline")}
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {outgoing.length > 0 ? (
        <section aria-labelledby="friends-outgoing">
          <h2 id="friends-outgoing" className="friends-section-title">
            {t("pending.outgoing")}
          </h2>
          <ul className="friend-list">
            {outgoing.map((request) => (
              <li key={request.user.id} className="friend-row">
                <PresenceAvatar
                  name={request.user.name}
                  url={request.user.avatar_url}
                  status="x"
                />
                <span className="friend-row__name">{request.user.name}</span>
                <span className="friend-row__actions">
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={withdraw.isPending}
                    onClick={() => withdraw.mutate(request.user.id)}
                  >
                    {t("action.withdraw")}
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </>
  );
}

function AddFriend() {
  const t = useT(friendsCatalog);
  const id = useId();
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const send = useSendFriendRequest();
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!code.trim()) return;
    setError(null);
    send.mutate(
      { code: code.trim() },
      {
        onSuccess: (answer) => {
          setCode("");
          notify(
            answer.status === "friends"
              ? t("add.friends", { name: answer.peer.name })
              : t("add.sent", { name: answer.peer.name }),
          );
        },
        onError: (err) => {
          if (err instanceof ApiError && err.fields.code)
            setError(fieldMessages(err.fields).code ?? null);
        },
      },
    );
  };
  return (
    <form className="friend-add" onSubmit={submit} noValidate>
      <h2>{t("add.title")}</h2>
      <FormField
        id={id}
        label={t("add.label")}
        hint={t("add.hint")}
        error={error}
      >
        <div className="friend-add__row">
          <Input
            id={id}
            value={code}
            autoComplete="off"
            spellCheck={false}
            placeholder="K7QX-M2PD" // i18n-ok: the shape of a code
            maxLength={16}
            invalid={Boolean(error)}
            onChange={(event) => {
              setCode(event.target.value);
              setError(null);
            }}
          />
          <Button
            type="submit"
            variant="primary"
            disabled={send.isPending || !code.trim()}
          >
            {t("add.submit")}
          </Button>
        </div>
      </FormField>
    </form>
  );
}
