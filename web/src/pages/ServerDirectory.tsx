// The server settings section "Verzeichnis" (discovery, v0.10): the owner's
// listing form with a preview, and the join requests, which the owner and
// `moderate_members` answer.

import { useId, useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { apiErrorMessage } from "../auth/rules.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Avatar } from "../components/Avatar.tsx";
import { FormError } from "../components/FormError.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { fieldDescription } from "../components/ui/fieldDescription.ts";
import { FormField } from "../components/ui/FormField.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { RadioCard } from "../components/ui/RadioCard.tsx";
import { Select } from "../components/ui/Select.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import { Textarea } from "../components/ui/Textarea.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import {
  useAnswerJoinRequest,
  useJoinRequests,
  useListingSettings,
  useSaveListingSettings,
} from "../discover/queries.ts";
import {
  DESCRIPTION_MAX,
  validateDescription,
  validateLanguage,
} from "../discover/rules.ts";
import type {
  DirectoryLanguage,
  JoinMode,
  JoinRequest,
  Listing,
  ListingSettings,
} from "../discover/types.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { serverSettings } from "../i18n/catalogs/serverSettings.ts";
import { formatWhen } from "../operator/format.ts";
import type { ServerDetail } from "../servers/types.ts";
import { DiscoverRow } from "./DiscoverPage.tsx";
import "./serverDirectory.css";

/** The listing form: the owner's alone. */
export function ListingSettingsPanel({
  server,
  languages,
}: {
  server: ServerDetail;
  languages: readonly DirectoryLanguage[];
}) {
  const t = useT(serverSettings);
  const { data, error, isFetching, refetch } = useListingSettings(
    server.id,
    true,
  );
  if (!data)
    return error ? (
      <LoadError
        message={t("directory.loadFailed")}
        pending={isFetching}
        onRetry={() => void refetch()}
      />
    ) : (
      <p className="settings-hint">{t("directory.loading")}</p>
    );
  return (
    <ListingForm
      // A saved answer starts the form again from what the server holds.
      key={JSON.stringify(data)}
      server={server}
      settings={data}
      languages={languages}
    />
  );
}

type Form = {
  listed: boolean;
  description: string;
  language: string;
  join_mode: JoinMode;
};

const formOf = (settings: ListingSettings): Form => ({
  listed: settings.listed,
  description: settings.description,
  language: settings.language ?? "",
  join_mode: settings.join_mode,
});

/** The description is one line: pasted line breaks become spaces. */
const oneLine = (value: string) => value.replace(/[\r\n]+/g, " ");

function ListingForm({
  server,
  settings,
  languages,
}: {
  server: ServerDetail;
  settings: ListingSettings;
  languages: readonly DirectoryLanguage[];
}) {
  const t = useT(serverSettings);
  const tc = useT(common);
  const ids = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const [form, setForm] = useState(() => formOf(settings));
  const { errors, clearField, setFields, fromError } = useFormErrors(formRef);
  const save = useSaveListingSettings(server.id);
  const status = useInlineStatus();
  const saved = formOf(settings);
  const dirty =
    form.listed !== saved.listed ||
    form.description.trim() !== saved.description ||
    form.language !== saved.language ||
    form.join_mode !== saved.join_mode;
  const unchanged = !dirty && !save.isPending && !status.status;
  const unchangedId = `${ids}-unchanged`;
  const descriptionId = `${ids}-description`;
  const countId = `${ids}-count`;
  const languageId = `${ids}-language`;
  const length = Array.from(form.description.trim()).length;

  const edit = <K extends keyof Form>(field: K, value: Form[K]) => {
    setForm((current) => ({ ...current, [field]: value }));
    clearField(field);
    if (field === "listed") clearField("description");
  };

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    const description = validateDescription(form.description, form.listed);
    if (description === "required")
      fields.description = t("directory.descriptionRequired");
    else if (description === "invalid")
      fields.description = t("directory.descriptionInvalid");
    else if (description)
      fields.description = t("directory.descriptionTooLong");
    if (validateLanguage(form.language, languages))
      fields.language = t("directory.languageRequired");
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    status.clear();
    save.mutate(
      {
        listed: form.listed,
        description: form.description.trim(),
        language: form.language,
        join_mode: form.join_mode,
      },
      {
        onSuccess: () => status.show({ tone: "success", message: tc("saved") }),
        onError: fromError,
      },
    );
  };

  const preview: Listing = {
    server_id: server.id,
    name: server.name,
    icon_url: server.icon_url ?? null,
    description: form.description.trim(),
    language: form.language,
    member_count: server.members.length,
    join_mode: form.join_mode,
    member: false,
    request: null,
  };
  const descriptionError = errors.fields.description;

  return (
    <form
      ref={formRef}
      onSubmit={onSubmit}
      noValidate
      className="settings-form server-directory-form"
    >
      {settings.delisted ? (
        <p className="server-directory-delisted">{t("directory.delisted")}</p>
      ) : null}
      <Switch
        label={t("directory.listed")}
        checked={form.listed}
        onChange={(event) => edit("listed", event.target.checked)}
      />
      <FormField
        id={descriptionId}
        label={t("directory.description")}
        error={descriptionError}
      >
        <Textarea
          id={descriptionId}
          rows={3}
          value={form.description}
          onChange={(event) => edit("description", oneLine(event.target.value))}
          invalid={Boolean(descriptionError) || length > DESCRIPTION_MAX}
          aria-describedby={fieldDescription(descriptionId, {
            error: descriptionError,
            extra: countId,
          })}
          className="resize-y"
        />
        <p
          id={countId}
          className={length > DESCRIPTION_MAX ? "ui-field-error" : "ui-hint"}
        >
          {t("directory.descriptionCount", {
            // Strings: counts stay unformatted, as on the profile.
            length: String(length),
            max: String(DESCRIPTION_MAX),
          })}
        </p>
      </FormField>
      <FormField
        id={languageId}
        label={t("directory.language")}
        error={errors.fields.language}
      >
        <Select
          id={languageId}
          value={form.language}
          onChange={(event) => edit("language", event.target.value)}
          invalid={Boolean(errors.fields.language)}
          aria-describedby={fieldDescription(languageId, {
            error: errors.fields.language,
          })}
        >
          <option value="" disabled>
            {t("directory.languagePick")}
          </option>
          {languages.map((language) => (
            <option
              key={language.code}
              value={language.code}
              lang={language.code}
            >
              {language.name}
            </option>
          ))}
        </Select>
      </FormField>
      <fieldset className="server-directory-modes">
        <legend className="ui-label">{t("directory.joinMode")}</legend>
        <RadioCard
          name={`${ids}-join-mode`}
          value="open"
          checked={form.join_mode === "open"}
          onChange={() => edit("join_mode", "open")}
          label={t("directory.joinOpen")}
          description={t("directory.joinOpenHint")}
        />
        <RadioCard
          name={`${ids}-join-mode`}
          value="request"
          checked={form.join_mode === "request"}
          onChange={() => edit("join_mode", "request")}
          label={t("directory.joinRequest")}
          description={t("directory.joinRequestHint")}
        />
      </fieldset>
      <div className="server-directory-preview">
        <h3 className="ui-label">{t("directory.preview")}</h3>
        {/* What others see in Entdecken; its buttons do nothing here. */}
        <ul className="discover-list" inert>
          <DiscoverRow
            listing={preview}
            languages={languages}
            onRequest={() => undefined}
          />
        </ul>
      </div>
      <FormError message={errors.form} />
      <div className="settings-actions">
        <SubmitButton
          pending={save.isPending}
          pendingLabel={t("directory.saving")}
          disabled={!dirty}
          aria-describedby={unchanged ? unchangedId : undefined}
        >
          {tc("save")}
        </SubmitButton>
        {unchanged ? (
          <span id={unchangedId} className="settings-hint">
            {t("directory.unchanged")}
          </span>
        ) : null}
        <InlineStatus status={status.status} />
      </div>
    </form>
  );
}

/** "Beitrittsanfragen": the owner's and `moderate_members`'. */
export function JoinRequestQueue({ server }: { server: ServerDetail }) {
  const t = useT(serverSettings);
  const headingId = useId();
  const {
    data: requests,
    isPending,
    error,
    isFetching,
    refetch,
  } = useJoinRequests(server.id, true);
  const tc = useT(common);
  const answer = useAnswerJoinRequest(server.id);
  const status = useInlineStatus();
  // When the section opened: "vor 2 Stunden" needs no live clock.
  const [now] = useState(Date.now);
  const respond = (request: JoinRequest, accept: boolean) => {
    status.clear();
    answer.mutate(
      { userId: request.user_id, accept },
      {
        onError: (failure) =>
          status.show({
            tone: "error",
            message:
              failure instanceof ApiError
                ? apiErrorMessage(failure)
                : tc("notSaved"),
          }),
      },
    );
  };
  const busy = (request: JoinRequest, accept: boolean) =>
    answer.isPending &&
    answer.variables?.userId === request.user_id &&
    answer.variables.accept === accept;

  return (
    <div className="server-directory-requests">
      <div className="server-directory-requests__head">
        <h3 id={headingId}>{t("joinRequests.title")}</h3>
        <InlineStatus status={status.status} />
      </div>
      {isPending ? (
        <p className="settings-hint">{t("joinRequests.loading")}</p>
      ) : error ? (
        <LoadError
          message={t("joinRequests.loadFailed")}
          pending={isFetching}
          onRetry={() => void refetch()}
        />
      ) : requests.length === 0 ? (
        <p className="settings-hint">{t("joinRequests.empty")}</p>
      ) : (
        <ul className="server-settings-list" aria-labelledby={headingId}>
          {requests.map((request) => (
            <li key={request.user_id} className="server-settings-row">
              <Avatar name={request.name} url={request.avatar_url} />
              <div className="server-settings-row__main">
                <span className="server-settings-row__name">
                  {request.name}
                  <span className="server-directory-requests__when">
                    {" · "}
                    <time dateTime={request.created_at}>
                      {formatWhen(request.created_at, now)}
                    </time>
                  </span>
                </span>
                {request.message ? (
                  <span className="server-directory-requests__message">
                    {request.message}
                  </span>
                ) : null}
              </div>
              <div className="server-settings-row__actions">
                <Button
                  size="sm"
                  variant="primary"
                  disabled={answer.isPending}
                  pending={busy(request, true)}
                  pendingLabel={t("joinRequests.accept")}
                  onClick={() => respond(request, true)}
                >
                  {t("joinRequests.accept")}
                </Button>
                <Button
                  size="sm"
                  disabled={answer.isPending}
                  pending={busy(request, false)}
                  pendingLabel={t("joinRequests.decline")}
                  onClick={() => respond(request, false)}
                >
                  {t("joinRequests.decline")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
