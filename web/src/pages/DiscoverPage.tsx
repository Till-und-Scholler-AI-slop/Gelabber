// "Entdecken": the servers their owners put in the directory, with a search
// and a language filter (reference docs/design/v0.10/directory.png).

import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useId, useState, type CSSProperties } from "react";

import { RequestJoinDialog } from "../components/DirectoryDialogs.tsx";
import { SearchIcon } from "../components/Icons.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { UserPanel } from "../components/UserPanel.tsx";
import { tileInitials, tileTint } from "../components/initials.ts";
import { notifyError } from "../components/toasts.ts";
import { useDebouncedValue } from "../components/useDebouncedValue.ts";
import { Button } from "../components/ui/Button.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { Input } from "../components/ui/Input.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import {
  useDirectoryStatus,
  useJoinListing,
  useListings,
} from "../discover/queries.ts";
import {
  QUERY_MAX,
  SEARCH_DEBOUNCE_MS,
  listingAction,
  validateQuery,
} from "../discover/rules.ts";
import type { DiscoverSearch } from "../discover/search.ts";
import type { DirectoryLanguage, Listing } from "../discover/types.ts";
import { useT } from "../i18n/catalog.ts";
import { discovery } from "../i18n/catalogs/discovery.ts";
import "./settings.css";
import "./discover.css";

/** The workspace sidebar on `/discover`: the language filter. */
export function DiscoverSidebar() {
  const t = useT(discovery);
  const headingId = useId();
  const navigate = useNavigate();
  const { lang } = useSearch({ strict: false }) as DiscoverSearch;
  const { data: status } = useDirectoryStatus();
  const languages = (status?.languages ?? []).filter(
    (language) => language.listings > 0,
  );
  const pick = (code: string | undefined) =>
    void navigate({
      to: "/discover",
      search: (previous: DiscoverSearch) => ({ ...previous, lang: code }),
      replace: true,
    });
  return (
    <aside
      className="channel-sidebar settings-sidebar"
      aria-label={t("sidebar.label")}
    >
      <div className="sidebar-brand">{t("page.title")}</div>
      <div className="settings-sidebar-scroll">
        <div className="settings-nav">
          <div className="settings-nav-group">
            <h2 className="settings-nav-heading" id={headingId}>
              {t("sidebar.language")}
            </h2>
            <ul aria-labelledby={headingId}>
              <li>
                <LanguageButton
                  label={t("sidebar.all")}
                  pressed={lang === undefined}
                  onClick={() => pick(undefined)}
                />
              </li>
              {languages.map((language) => (
                <li key={language.code}>
                  <LanguageButton
                    label={language.name}
                    lang={language.code}
                    pressed={lang === language.code}
                    onClick={() => pick(language.code)}
                  />
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>
      <UserPanel />
    </aside>
  );
}

function LanguageButton({
  label,
  lang,
  pressed,
  onClick,
}: {
  label: string;
  /** The endonym's own language, for its pronunciation. */
  lang?: string;
  pressed: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="settings-nav-link discover-language"
      aria-pressed={pressed}
      lang={lang}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

export function DiscoverPage({ q, lang }: DiscoverSearch) {
  const t = useT(discovery);
  const navigate = useNavigate();
  const [input, setInput] = useState(q ?? "");
  const typed = useDebouncedValue(input.trim(), SEARCH_DEBOUNCE_MS);
  // The search reaches the address once typing pauses; the list follows it.
  useEffect(() => {
    if (typed === (q ?? "") || validateQuery(typed)) return;
    void navigate({
      to: "/discover",
      search: (previous: DiscoverSearch) => ({
        ...previous,
        q: typed || undefined,
      }),
      replace: true,
    });
  }, [typed, q, navigate]);
  const { data: status } = useDirectoryStatus();
  const listings = useListings(q ?? "", lang);
  const [requesting, setRequesting] = useState<Listing | null>(null);
  const rows = listings.data?.pages.flatMap((page) => page.listings);

  return (
    <div className="discover-screen">
      {/* The shell's header line, empty here as in the reference. */}
      <div className="discover-header" aria-hidden="true" />
      <div className="discover-scroll">
        <section className="discover-page" aria-labelledby="discover-title">
          <header className="discover-page-head">
            <h1 id="discover-title">{t("page.title")}</h1>
            <p>{t("page.lead")}</p>
          </header>
          <div className="discover-search">
            <SearchIcon size={16} aria-hidden="true" />
            <Input
              type="search"
              aria-label={t("page.search")}
              placeholder={t("page.search")}
              maxLength={QUERY_MAX}
              value={input}
              onChange={(event) => setInput(event.target.value)}
            />
          </div>
          {listings.isError && !rows ? (
            <LoadError
              message={t("page.loadFailed")}
              pending={listings.isFetching}
              onRetry={() => void listings.refetch()}
            />
          ) : !rows ? (
            <SkeletonRows count={4} className="discover-skeleton" />
          ) : rows.length === 0 ? (
            <p className="discover-empty">{t("page.empty")}</p>
          ) : (
            <>
              <ul className="discover-list" aria-label={t("page.listLabel")}>
                {rows.map((listing) => (
                  <DiscoverRow
                    key={listing.server_id}
                    listing={listing}
                    languages={status?.languages ?? []}
                    onRequest={setRequesting}
                  />
                ))}
              </ul>
              {listings.hasNextPage ? (
                <div className="discover-more">
                  <Button
                    pending={listings.isFetchingNextPage}
                    pendingLabel={t("page.more")}
                    onClick={() => void listings.fetchNextPage()}
                  >
                    {t("page.more")}
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </section>
      </div>
      <RequestJoinDialog
        listing={requesting}
        onClose={() => setRequesting(null)}
      />
    </div>
  );
}

export function DiscoverRow({
  listing,
  languages,
  onRequest,
}: {
  listing: Listing;
  languages: readonly DirectoryLanguage[];
  onRequest: (listing: Listing) => void;
}) {
  const t = useT(discovery);
  const nameId = useId();
  const language =
    languages.find((entry) => entry.code === listing.language)?.name ??
    listing.language;
  return (
    <li className="discover-row">
      <span
        aria-hidden="true"
        className="discover-tile"
        style={
          { "--server-tile": tileTint(listing.server_id) } as CSSProperties
        }
      >
        <ServerIconFace
          url={listing.icon_url}
          fallback={tileInitials(listing.name)}
        />
      </span>
      <div className="discover-row-text">
        <h2 id={nameId}>{listing.name}</h2>
        <p className="discover-row-description">{listing.description}</p>
        <p className="discover-row-meta">
          {t("row.meta", {
            language,
            members: t("row.members", { count: listing.member_count }),
          })}
        </p>
      </div>
      <ListingButton
        listing={listing}
        describedBy={nameId}
        onRequest={() => onRequest(listing)}
      />
    </li>
  );
}

function ListingButton({
  listing,
  describedBy,
  onRequest,
}: {
  listing: Listing;
  /** The row's name, so "Beitreten" says which server. */
  describedBy: string;
  onRequest: () => void;
}) {
  const t = useT(discovery);
  const navigate = useNavigate();
  const join = useJoinListing();
  switch (listingAction(listing)) {
    case "open":
      return (
        <Link
          to="/s/$serverId"
          params={{ serverId: listing.server_id }}
          aria-describedby={describedBy}
          className={buttonClass({ className: "discover-action" })}
        >
          {t("actions.open")}
        </Link>
      );
    case "join":
      return (
        <Button
          variant="primary"
          className="discover-action"
          aria-describedby={describedBy}
          pending={join.isPending}
          pendingLabel={t("actions.joining")}
          onClick={() =>
            join.mutate(listing.server_id, {
              onSuccess: (server) =>
                void navigate({
                  to: "/s/$serverId",
                  params: { serverId: server.id },
                }),
              onError: (error) => notifyError(error),
            })
          }
        >
          {t("actions.join")}
        </Button>
      );
    case "request":
      return (
        <Button
          className="discover-action"
          aria-describedby={describedBy}
          onClick={onRequest}
        >
          {t("actions.request")}
        </Button>
      );
    case "requested":
      return (
        <Button
          className="discover-action"
          aria-describedby={describedBy}
          disabled
        >
          {t("actions.requested")}
        </Button>
      );
    case "declined":
      return (
        <Button
          className="discover-action"
          aria-describedby={describedBy}
          disabled
        >
          {t("actions.declined")}
        </Button>
      );
  }
}
