import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { useSession } from "../auth/session.ts";
import { useBlockedIds } from "../blocks/queries.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import { AttachmentList } from "../components/Attachments.tsx";
import { Avatar } from "../components/Avatar.tsx";
import { placeholderColour } from "../components/imageBox.ts";
import {
  ArrowRightIcon,
  ChatIcon,
  CheckIcon,
  EyeIcon,
  FileIcon,
  HashIcon,
  SearchIcon,
  SpeakerIcon,
} from "../components/Icons.tsx";
import { InviteDialog } from "../components/InviteDialog.tsx";
import { MemberPanel, MemberPanelToggle } from "../components/MemberPanel.tsx";
import {
  mentionRenderer,
  messagePing,
} from "../components/mentionRenderer.tsx";
import { formatFull, formatWhen } from "../components/messageTime.ts";
import { isMac, keyNames } from "../components/shellShortcuts.ts";
import { openSwitcher } from "../components/switcherOpen.ts";
import { Button } from "../components/ui/Button.tsx";
import { FieldShell } from "../components/ui/FieldShell.tsx";
import { Input } from "../components/ui/Input.tsx";
import { useT } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import { useCurrentLocale } from "../i18n/locale.ts";
import { attachmentUrl, listMessages, thumbnailUrl } from "../messages/api.ts";
import { MessageContent } from "../messages/MessageContent.tsx";
import { plainText } from "../messages/markdown.ts";
import {
  readableMentions,
  toWire,
  type MentionMember,
} from "../messages/mentions.ts";
import { previewText } from "../messages/notify.ts";
import { requestMessageJump } from "../messages/messageJump.ts";
import { useSendMessage } from "../messages/queries.ts";
import { useMarkManyRead, useReadState } from "../messages/readState.ts";
import { formatSize } from "../messages/rules.ts";
import { asSystem, systemText } from "../messages/system.ts";
import type { Message } from "../messages/types.ts";
import { useBadges } from "../messages/useBadge.ts";
import {
  greetingKey,
  isPicture,
  mentionsMe,
  missedCards,
  SHARED_ASKED,
  sharedShown,
  shownCount,
  unreadTail,
  type MissedCard,
} from "../servers/overview.ts";
import { can } from "../servers/permissions.ts";
import { listSharedFiles, type SharedFile } from "../servers/sharedFiles.ts";
import type { ServerDetail } from "../servers/types.ts";
import { EMPTY_LIVE, useVoiceRoster } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { useLiveWatchers } from "../voice/useLiveWatchers.ts";
import { joinToWatch } from "../voice/watchLive.ts";
import "./overview.css";

/**
 * The server's front door (GEL-69): only what is shown nowhere else. The
 * sidebar has the channels and who is in a call, the member list who is
 * online, the call bar the call; here are the Lives, what was missed and
 * what was shared.
 */
export function CommunityOverview({ server }: { server: ServerDetail }) {
  const t = useT(servers);
  const missed = useMissed(server);
  return (
    <div className="community-overview-layout">
      <section className="community-overview" aria-label={t("overview.label")}>
        <div className="community-overview__bar">
          <ChatIcon size={18} />
          <span className="community-overview__title">
            {t("overview.label")}
          </span>
          <JumpField />
          <MemberPanelToggle />
        </div>
        <div className="community-overview__body">
          <div className="overview-main">
            <Greeting server={server} missed={missed} />
            <Streams server={server} />
            {missed.cards.length > 0 ? (
              <Missed server={server} missed={missed} />
            ) : null}
          </div>
          <Shared server={server} shownMessages={missed.shownMessages} />
        </div>
      </section>
      <MemberPanel server={server} />
    </div>
  );
}

/** "Springen zu …": the quick switcher, as Strg/⌘+K opens it. */
function JumpField() {
  const t = useT(servers);
  const { mod } = keyNames(isMac());
  return (
    <button
      type="button"
      className="overview-jump"
      onClick={openSwitcher}
      aria-keyshortcuts={isMac() ? "Meta+K" : "Control+K"}
    >
      <SearchIcon size={15} />
      <span className="overview-jump__label">{t("overview.search")}</span>
      <kbd>
        {mod} K{/* i18n-ok */}
      </kbd>
    </button>
  );
}

/** One line: the greeting, and "Alles gelesen" beside it once the read
 * state says nothing was missed (GEL-75): no empty "Verpasst" box. */
function Greeting({
  server,
  missed,
}: {
  server: ServerDetail;
  missed: MissedState;
}) {
  const t = useT(servers);
  const name = useSession((s) => s.user?.name ?? "");
  // The hour as the page was drawn; a new visit greets anew.
  const [hour] = useState(() => new Date().getHours());
  const allRead = missed.loaded && missed.cards.length === 0;
  return (
    <div className="overview-hello">
      <div className="overview-hello__line">
        <h1 className="overview-greeting">{t(greetingKey(hour), { name })}</h1>
        {allRead ? (
          <span className="overview-all-read">
            <CheckIcon size={14} />
            {t("overview.allRead")}
          </span>
        ) : null}
      </div>
      {allRead && server.members.length === 1 ? (
        <Alone server={server} />
      ) : null}
    </div>
  );
}

/** Alone in the server: a hint and the way to invite someone. */
function Alone({ server }: { server: ServerDetail }) {
  const t = useT(servers);
  const [inviting, setInviting] = useState(false);
  return (
    <div className="overview-alone">
      <p>{t("overview.alone")}</p>
      <Button size="sm" onClick={() => setInviting(true)}>
        {t("overview.invite")}
      </Button>
      <InviteDialog
        server={server}
        open={inviting}
        onClose={() => setInviting(false)}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Lives

type Live = { channelId: string; userId: string };

/** Everyone in the server live with a screen, in the order they went live. */
function Streams({ server }: { server: ServerDetail }) {
  const t = useT(servers);
  const lives = useVoiceRoster((s) => s.live[server.id] ?? EMPTY_LIVE);
  const known = new Set(server.channels.map((c) => c.id));
  const entries: Live[] = Object.entries(lives).flatMap(
    ([channelId, people]) =>
      known.has(channelId)
        ? Object.entries(people)
            .filter(([, kinds]) => kinds.s === true)
            .map(([userId]) => ({ channelId, userId }))
        : [],
  );
  if (entries.length === 0) return null;
  return (
    <section className="overview-streams" aria-label={t("overview.streams")}>
      {entries.map((live) => (
        <StreamCard
          key={`${live.channelId}/${live.userId}`}
          server={server}
          live={live}
        />
      ))}
    </section>
  );
}

function StreamCard({ server, live }: { server: ServerDetail; live: Live }) {
  const t = useT(servers);
  const navigate = useNavigate();
  const me = useUserId();
  const own = live.userId === me;
  const member = server.members.find((m) => m.user_id === live.userId);
  const name = member?.name ?? "";
  const channel = server.channels.find((c) => c.id === live.channelId);
  // The picture only where the call already has it: my own screen, or a
  // Live I watch in the call I am in. Outside, the avatar stands in.
  const stream = useVoice((s) =>
    s.status === "joined" &&
    s.serverId === server.id &&
    s.channelId === live.channelId
      ? own
        ? s.localScreen
        : (s.remote[live.userId]?.s ?? null)
      : null,
  );
  const here = useVoice(
    (s) =>
      s.status === "joined" &&
      s.serverId === server.id &&
      s.channelId === live.channelId,
  );
  const watchers = useLiveWatchers({
    serverId: server.id,
    channelId: live.channelId,
    owner: live.userId,
    members: server.members,
  }).watching.length;
  const open = () => {
    void navigate({
      to: "/s/$serverId/c/$channelId",
      params: { serverId: server.id, channelId: live.channelId },
    });
    // Outside a call this joins it with the Live watched from the start;
    // in this call it watches; in another one only the page opens.
    if (!own && !here && can(server, "join_voice"))
      joinToWatch({
        serverId: server.id,
        channelId: live.channelId,
        channelName: channel?.name ?? null,
        people: [live.userId],
      });
  };
  const label = own ? t("overview.ownScreen") : t("overview.watch", { name });
  return (
    <button
      type="button"
      className="overview-stream"
      onClick={open}
      aria-label={label}
    >
      <span className="overview-stream__picture">
        {stream ? (
          <StreamVideo stream={stream} />
        ) : (
          <span className="overview-stream__stand-in">
            <Avatar
              name={name}
              url={member?.avatar_url ?? null}
              size="lg"
              decorative
            />
          </span>
        )}
        <span className="overview-stream__live" aria-hidden="true">
          {t("overview.live")}
        </span>
        <span
          className="overview-stream__viewers"
          title={t("overview.viewers", { count: watchers })}
        >
          <EyeIcon size={14} />
          <span aria-hidden="true">{watchers}</span>
          <span className="sr-only">
            {t("overview.viewers", { count: watchers })}
          </span>
        </span>
      </span>
      {/* Flat (GEL-75): the picture small on the left, who and where
          beside it. The whole card stays one button. */}
      <span className="overview-stream__info">
        <span className="overview-stream__by">
          <Avatar name={name} url={member?.avatar_url ?? null} decorative />
          <span className="overview-stream__name">
            {own ? t("overview.you") : name}
          </span>
        </span>
        {channel ? (
          <span className="overview-stream__channel">
            <SpeakerIcon size={13} />
            <span>{channel.name}</span>
          </span>
        ) : null}
        <span
          className="overview-stream__go"
          data-own={own ? "" : undefined}
          aria-hidden="true"
        >
          {own ? null : <EyeIcon size={14} />}
          {own ? t("overview.open") : t("overview.watchShort")}
        </span>
      </span>
    </button>
  );
}

/** A Live's picture, muted: its sound belongs to the call. */
function StreamVideo({ stream }: { stream: MediaStream }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.srcObject = stream;
    void el.play().catch(() => {});
    return () => {
      el.srcObject = null;
    };
  }, [stream]);
  return <video ref={ref} muted autoPlay playsInline aria-hidden="true" />;
}

// ---------------------------------------------------------------------------
// Verpasst

type MissedState = {
  cards: MissedCard[];
  hidden: number;
  alone: boolean;
  messages: ReadonlyMap<string, Message[] | undefined>;
  /** Messages the cards show: "Geteilt" leaves their files out. */
  shownMessages: ReadonlySet<string>;
  loaded: boolean;
};

/** The unread channels and their newest unread messages. A new message
 * changes the channel's newest order, and its card asks again. */
function useMissed(server: ServerDetail): MissedState {
  const userId = useUserId();
  const generation = scopeGeneration();
  const badges = useBadges();
  const { data } = useReadState();
  const rows = useMemo(
    () => new Map((data ?? []).map((row) => [row.channel_id, row])),
    [data],
  );
  const { cards, hidden } = missedCards(server.channels, rows, badges);
  const alone = cards.length === 1;
  const results = useQueries({
    queries: cards.map((card) => {
      const count = shownCount(card, alone);
      return {
        queryKey: [
          "user",
          userId,
          generation,
          "overview-missed",
          card.channel.id,
          card.latest,
          card.readOrder,
          count,
        ],
        queryFn: async ({ signal }: { signal: AbortSignal }) =>
          unreadTail(
            (await listMessages(card.channel.id, { limit: count }, signal))
              .messages,
            card.readOrder,
            count,
          ),
        enabled: Boolean(userId),
        staleTime: 30_000,
        retry: false,
        // The old messages stay until the new ones are there.
        placeholderData: keepPreviousData,
      };
    }),
  });
  const messages = new Map(
    cards.map((card, n) => [card.channel.id, results[n]?.data]),
  );
  const shownMessages = new Set(
    [...messages.values()].flatMap((list) => (list ?? []).map((m) => m.id)),
  );
  return {
    cards,
    hidden,
    alone,
    messages,
    shownMessages,
    loaded: data !== undefined,
  };
}

function Missed({
  server,
  missed,
}: {
  server: ServerDetail;
  missed: MissedState;
}) {
  const t = useT(servers);
  const heading = useId();
  const blocked = useBlockedIds();
  // Kept by channel: a card that moves down keeps what was typed in it.
  const [drafts, setDrafts] = useState<ReadonlyMap<string, string>>(
    () => new Map(),
  );
  const members: MentionMember[] = server.members.map((member) => ({
    id: member.user_id,
    name: member.name,
  }));
  return (
    <section className="overview-missed" aria-labelledby={heading}>
      <h2 id={heading} className="overview-heading">
        {t("overview.missed")}
      </h2>
      <div className="overview-cards">
        {missed.cards.map((card, n) => (
          <MissedChannel
            key={card.channel.id}
            server={server}
            card={card}
            messages={missed.messages
              .get(card.channel.id)
              // As the chat folds them: whoever the user blocks is left
              // out here, and counted in "x weitere".
              ?.filter((message) => !blocked.has(message.author.id))}
            alone={missed.alone}
            reply={
              n === 0
                ? {
                    text: drafts.get(card.channel.id) ?? "",
                    setText: (text) =>
                      setDrafts((old) =>
                        new Map(old).set(card.channel.id, text),
                      ),
                  }
                : null
            }
            members={members}
          />
        ))}
        {missed.hidden > 0 ? (
          <p className="overview-cards__hidden">
            {t("overview.moreChannels", { count: missed.hidden })}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function MissedChannel({
  server,
  card,
  messages,
  alone,
  reply,
  members,
}: {
  server: ServerDetail;
  card: MissedCard;
  messages: Message[] | undefined;
  alone: boolean;
  /** Only the first card has a reply field: its text, kept by channel. */
  reply: { text: string; setText: (text: string) => void } | null;
  members: readonly MentionMember[];
}) {
  const t = useT(servers);
  const markRead = useMarkManyRead();
  const heading = useId();
  const { channel } = card;
  const to = {
    to: "/s/$serverId/c/$channelId",
    params: { serverId: server.id, channelId: channel.id },
  } as const;
  // Until the messages are there, how many more is not known yet.
  const more = messages ? Math.max(0, card.unread - messages.length) : 0;
  const name = (
    <Link {...to} id={heading} className="overview-card__name">
      <HashIcon size={16} />
      <span>{channel.name}</span>
    </Link>
  );
  const count = (
    <span className="overview-card__new">
      {t("overview.new", { count: card.unread })}
    </span>
  );
  const actions = (
    <span className="overview-card__actions">
      <button
        type="button"
        className="overview-card__action"
        onClick={() =>
          // Failed, the card stays: the chat's "Gelesen" does the same.
          void markRead({ channel: channel.id }).catch(() => {})
        }
      >
        <CheckIcon size={15} />
        {t("overview.markRead")}
      </button>
      <Link {...to} className="overview-card__action">
        {t("overview.open")}
        <ArrowRightIcon size={15} />
      </Link>
    </span>
  );
  // A busy channel without a mention is one line (GEL-75): its name, the
  // count, the newest message and the two actions.
  if (card.busy && !alone) {
    const last = messages?.at(-1);
    return (
      <section
        className="overview-card overview-card--line"
        data-busy=""
        aria-labelledby={heading}
      >
        <div className="overview-card__head">
          {name}
          {count}
          <span className="overview-card__preview">
            {last ? (
              <>
                <span className="overview-card__preview-author">
                  {last.author.name}:
                </span>{" "}
                {linePreview(last, members)}
              </>
            ) : null}
          </span>
          {actions}
        </div>
      </section>
    );
  }
  return (
    <section className="overview-card" aria-labelledby={heading}>
      <div className="overview-card__head">
        {name}
        {card.mentioned ? (
          <span
            className="overview-card__at"
            role="img"
            aria-label={t("overview.mentioned")}
          >
            @
          </span>
        ) : null}
        {count}
        {actions}
      </div>
      {messages?.map((message) => (
        <OverviewMessage key={message.id} message={message} members={members} />
      ))}
      {more > 0 ? (
        <Link {...to} className="overview-card__more">
          {alone
            ? t("overview.older", { count: more })
            : t("overview.more", { count: more })}
          <ArrowRightIcon size={14} />
        </Link>
      ) : null}
      {reply && can(server, "send_messages") ? (
        <ReplyField
          channelId={channel.id}
          channelName={channel.name}
          members={members}
          text={reply.text}
          setText={reply.setText}
        />
      ) : null}
    </section>
  );
}

/** A message as one line of plain text, as a notification shows it. */
function linePreview(
  message: Message,
  members: readonly MentionMember[],
): string {
  const system = asSystem(message.system);
  if (system) return systemText(system, message.author.name);
  return previewText(
    readableMentions(plainText(message.content), members),
    (message.attachments ?? []).length > 0,
    Boolean(message.gif),
  );
}

/** A message as the chat shows it: avatar, name, time, text, files. */
function OverviewMessage({
  message,
  members,
}: {
  message: Message;
  members: readonly MentionMember[];
}) {
  const me = useUserId();
  const locale = useCurrentLocale();
  const renderMention = useMemo(
    () => mentionRenderer(members, me, message.content, messagePing(message)),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `locale` remakes it
    [members, me, message, locale],
  );
  const system = asSystem(message.system);
  return (
    <article
      className="overview-message"
      data-mentioned={mentionsMe(message, me) ? "" : undefined}
    >
      <Avatar name={message.author.name} url={message.author.avatar_url} />
      <div className="overview-message__body">
        <div className="overview-message__top">
          <span className="overview-message__author">
            {message.author.name}
          </span>
          <time
            dateTime={message.created_at}
            title={formatFull(message.created_at)}
          >
            {formatWhen(message.created_at)}
          </time>
        </div>
        {system ? (
          <p className="overview-message__text">
            {systemText(system, message.author.name)}
          </p>
        ) : message.content ? (
          <div className="overview-message__text">
            <MessageContent
              content={message.content}
              renderMention={renderMention}
            />
          </div>
        ) : null}
        <AttachmentList attachments={message.attachments ?? []} />
      </div>
    </article>
  );
}

/** "Antworten in #kanal": Enter sends, as in the chat. */
function ReplyField({
  channelId,
  channelName,
  members,
  text,
  setText,
}: {
  channelId: string;
  channelName: string;
  members: readonly MentionMember[];
  text: string;
  setText: (text: string) => void;
}) {
  const t = useT(servers);
  const user = useSession((s) => s.user);
  const send = useSendMessage(channelId, {
    id: user?.id ?? "",
    name: user?.name ?? "",
    avatar_url: user?.avatar_url ?? null,
  });
  const label = t("overview.reply", { channel: channelName });
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (
      event.key !== "Enter" ||
      event.shiftKey ||
      event.nativeEvent.isComposing
    )
      return;
    event.preventDefault();
    const content = text.trim();
    if (!content || !user) return;
    // A typed `@Name` of a member is sent as their mention, as the
    // composer sends it.
    send.mutate({ content: toWire(content, members) });
    setText("");
  };
  return (
    <FieldShell as="label" className="overview-reply">
      <span className="sr-only">{label}</span>
      <Input
        bare
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder={label}
        maxLength={4000}
      />
      <kbd aria-hidden="true">Enter{/* i18n-ok */}</kbd>
    </FieldShell>
  );
}

// ---------------------------------------------------------------------------
// Geteilt

/** Files sent in the server's text channels, newest first, leaving out
 * what "Verpasst" already shows. */
function Shared({
  server,
  shownMessages,
}: {
  server: ServerDetail;
  shownMessages: ReadonlySet<string>;
}) {
  const t = useT(servers);
  const heading = useId();
  const userId = useUserId();
  const generation = scopeGeneration();
  const { data: readState } = useReadState();
  // A new message anywhere in the server may carry a file.
  const newest = (readState ?? [])
    .filter((row) => row.server_id === server.id)
    .reduce((max, row) => Math.max(max, row.latest_order ?? 0), 0);
  const { data } = useQuery({
    queryKey: ["user", userId, generation, "shared-files", server.id, newest],
    queryFn: ({ signal }) => listSharedFiles(server.id, SHARED_ASKED, signal),
    enabled: Boolean(userId),
    staleTime: 30_000,
    retry: false,
    placeholderData: keepPreviousData,
  });
  const blocked = useBlockedIds();
  const files = sharedShown(
    (data ?? []).filter((file) => !blocked.has(file.author.id)),
    shownMessages,
  );
  if (files.length === 0) return null;
  const textChannels = server.channels.filter((c) => c.kind === "text");
  const names = new Map(server.channels.map((c) => [c.id, c.name]));
  const several = textChannels.length > 1;
  return (
    <aside className="overview-shared" aria-labelledby={heading}>
      <h2 id={heading} className="overview-heading">
        {t("overview.shared")}
      </h2>
      <ul className="overview-shared__list">
        {files.map((file) => (
          <li key={file.id}>
            <SharedItem
              server={server}
              file={file}
              channel={several ? (names.get(file.channel_id) ?? null) : null}
            />
          </li>
        ))}
      </ul>
    </aside>
  );
}

function SharedItem({
  server,
  file,
  channel,
}: {
  server: ServerDetail;
  file: SharedFile;
  /** The channel's name, where the server has more than one. */
  channel: string | null;
}) {
  const t = useT(servers);
  const navigate = useNavigate();
  const open = () => {
    requestMessageJump({
      channelId: file.channel_id,
      messageId: file.message_id,
    });
    void navigate({
      to: "/s/$serverId/c/$channelId",
      params: { serverId: server.id, channelId: file.channel_id },
    });
  };
  // One size for pictures and files (GEL-75): 160 × 100, never stretched,
  // who sent it and when below.
  return (
    <button type="button" className="overview-shared__tile" onClick={open}>
      {isPicture(file.content_type) ? (
        <span
          className="overview-shared__picture"
          style={{ backgroundColor: placeholderColour(file) }}
        >
          <img
            src={
              file.thumbnail
                ? thumbnailUrl(file.id, 400)
                : attachmentUrl(file.id)
            }
            alt={
              file.description ??
              t("overview.pictureBy", { name: file.author.name })
            }
            loading="lazy"
          />
          {channel ? (
            <span className="overview-shared__tag">#{channel}</span>
          ) : null}
        </span>
      ) : (
        <span className="overview-shared__file">
          <FileIcon size={22} />
          <span className="overview-shared__text">
            <span className="overview-shared__filename">{file.filename}</span>
            <span className="overview-shared__meta">
              {channel
                ? t("overview.fileMeta", {
                    name: `#${channel}`,
                    detail: formatSize(file.size),
                  })
                : formatSize(file.size)}
            </span>
          </span>
        </span>
      )}
      <span className="overview-shared__by">
        <Avatar
          name={file.author.name}
          url={file.author.avatar_url}
          decorative
        />
        <span className="overview-shared__name">{file.author.name}</span>
        <time dateTime={file.created_at} title={formatFull(file.created_at)}>
          {formatWhen(file.created_at)}
        </time>
      </span>
    </button>
  );
}
