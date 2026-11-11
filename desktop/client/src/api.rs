//! One typed function per REST route the web client calls (web/src/*/api.ts,
//! backend/api/src). Paths are relative to `/api`; query values and path
//! segments are encoded like the browser does.

use reqwest::Method;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::http::{ApiError, Http, encode_component, query};
use crate::models::*;

type Result<T> = std::result::Result<T, ApiError>;

/// Paging of `GET /api/channels/{id}/messages`; `before` and `after` take
/// `ts|uuid` cursors ([`Message::cursor`]), a message id or a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    pub before: Option<String>,
    pub after: Option<String>,
    /// 1..=100; the server defaults to 50.
    pub limit: Option<u32>,
}

impl HistoryQuery {
    pub fn latest(limit: u32) -> Self {
        Self {
            limit: Some(limit),
            ..Self::default()
        }
    }

    pub fn before(cursor: String, limit: u32) -> Self {
        Self {
            before: Some(cursor),
            limit: Some(limit),
            ..Self::default()
        }
    }

    pub fn after(cursor: String, limit: u32) -> Self {
        Self {
            after: Some(cursor),
            limit: Some(limit),
            ..Self::default()
        }
    }
}

impl Http {
    // ------------------------------------------------------------- auth

    /// `GET /api/auth/session`, typed. Never 401; `user: None` = signed out.
    pub async fn auth_session(&self) -> Result<SessionResponse> {
        let value = self.session().await?;
        serde_json::from_value(value).map_err(|e| ApiError::Decode(format!("/auth/session: {e}")))
    }

    /// `POST /api/auth/login`, typed; adopts the rotated CSRF token.
    pub async fn sign_in(&self, email: &str, password: &str) -> Result<SessionResponse> {
        if self.csrf_token().is_none() {
            self.session().await?;
        }
        self.send(
            Method::POST,
            "/auth/login",
            &json!({"email": email, "password": password}),
        )
        .await
    }

    /// `POST /api/auth/register`: `user` set = signed in; otherwise
    /// `verification_sent` says a mail went out.
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        name: &str,
    ) -> Result<RegisterResponse> {
        self.register_with(&RegisterBody::new(email, password, name))
            .await
    }

    pub async fn logout(&self) -> Result<CsrfResponse> {
        self.send_empty(Method::POST, "/auth/logout").await
    }

    pub async fn logout_all(&self) -> Result<CsrfResponse> {
        self.send_empty(Method::POST, "/auth/logout-all").await
    }

    /// Changes the password; this client gets a fresh session, all others
    /// are revoked.
    pub async fn change_password(
        &self,
        current_password: &str,
        password: &str,
    ) -> Result<SessionResponse> {
        self.send(
            Method::POST,
            "/auth/password",
            &json!({"current_password": current_password, "password": password}),
        )
        .await
    }

    /// `GET /api/auth/mail`: whether the instance sends account mail.
    pub async fn mail_status(&self) -> Result<MailStatus> {
        self.get("/auth/mail").await
    }

    pub async fn verify_email(&self, token: &str, password: &str) -> Result<SessionResponse> {
        self.send(
            Method::POST,
            "/auth/verify-email",
            &json!({"token": token, "password": password}),
        )
        .await
    }

    /// Always `202 {}` (also for unknown addresses); 404 without mail.
    pub async fn request_password_reset(&self, email: &str) -> Result<()> {
        let _: serde_json::Value = self
            .send(
                Method::POST,
                "/auth/password-reset",
                &json!({"email": email}),
            )
            .await?;
        Ok(())
    }

    pub async fn confirm_password_reset(
        &self,
        token: &str,
        password: &str,
    ) -> Result<SessionResponse> {
        self.send(
            Method::POST,
            "/auth/password-reset/confirm",
            &json!({"token": token, "password": password}),
        )
        .await
    }

    /// `GET /api/auth/providers`: the sign-in providers the instance offers.
    pub async fn auth_providers(&self) -> Result<Vec<AuthProvider>> {
        self.get::<AuthProviders>("/auth/providers")
            .await
            .map(|answer| answer.providers)
    }

    /// `POST /api/auth/oidc/{provider}/start`, desktop variant: the URL of
    /// the provider's sign-in page for the system browser. The flow in
    /// `oidc.rs` makes `desktop` and runs the rest.
    pub async fn oidc_start(
        &self,
        provider: &str,
        start: &OidcStart,
        desktop: &OidcDesktop,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(flatten)]
            start: &'a OidcStart,
            desktop: &'a OidcDesktop,
        }
        let answer: OidcStartResponse = self
            .send(
                Method::POST,
                &format!("/auth/oidc/{}/start", encode_component(provider)),
                &Body { start, desktop },
            )
            .await?;
        Ok(answer.url)
    }

    /// `POST /api/auth/oidc/desktop/exchange`: trades the code the loopback
    /// receiver got, proven by the flow's PKCE verifier. `signed_in` puts
    /// the session cookie into the jar and rotates CSRF, as [`Self::sign_in`].
    pub async fn oidc_exchange(&self, code: &str, verifier: &str) -> Result<OidcExchange> {
        self.send(
            Method::POST,
            "/auth/oidc/desktop/exchange",
            &json!({"code": code, "verifier": verifier}),
        )
        .await
    }

    /// `POST /api/auth/oidc/pending`: the first sign-in a name-step token
    /// stands for; does not use it up.
    pub async fn oidc_pending(&self, token: &str) -> Result<OidcPending> {
        self.send(Method::POST, "/auth/oidc/pending", &json!({"token": token}))
            .await
    }

    /// `POST /api/auth/oidc/complete` with the desktop flow's verifier:
    /// creates the account with its chosen name and signs it in.
    pub async fn oidc_complete(
        &self,
        token: &str,
        name: &str,
        verifier: &str,
    ) -> Result<OidcCompleteResponse> {
        self.send(
            Method::POST,
            "/auth/oidc/complete",
            // The language the app shows becomes the account's.
            &json!({
                "token": token,
                "name": name,
                "verifier": verifier,
                "locale": gelabber_i18n::get().as_str(),
            }),
        )
        .await
    }

    /// `GET /api/auth/methods`: password and providers of the account.
    pub async fn sign_in_methods(&self) -> Result<SignInMethods> {
        self.get("/auth/methods").await
    }

    /// Detaches a provider; `409 password_required` without a password.
    pub async fn disconnect_provider(&self, provider: &str) -> Result<SignInMethods> {
        self.send_empty(
            Method::DELETE,
            &format!("/auth/oidc/{}", encode_component(provider)),
        )
        .await
    }

    /// The first password of an account that signs in only through a
    /// provider (no `current_password`). Needs a recent sign-in
    /// (`403 reauth_required`); like [`Self::change_password`], this client
    /// gets a fresh session and all others are revoked.
    pub async fn set_password(&self, password: &str) -> Result<SessionResponse> {
        self.send(
            Method::POST,
            "/auth/password",
            &json!({"password": password}),
        )
        .await
    }

    /// `GET /api/auth/registration`: whether registering needs a code.
    pub async fn registration_mode(&self) -> Result<RegistrationMode> {
        self.get::<RegistrationStatus>("/auth/registration")
            .await
            .map(|status| status.mode)
    }

    /// [`Self::register`] with the whole body, e.g. with the codes that
    /// admit a new account on an invitation-only instance.
    pub async fn register_with(&self, body: &RegisterBody) -> Result<RegisterResponse> {
        if self.csrf_token().is_none() {
            self.session().await?;
        }
        self.send(Method::POST, "/auth/register", body).await
    }

    /// `GET /api/auth/sessions`: the account's live sessions, this one
    /// first, then by last use.
    pub async fn sessions(&self) -> Result<Vec<DeviceSession>> {
        self.get::<DeviceSessions>("/auth/sessions")
            .await
            .map(|answer| answer.sessions.0)
    }

    /// Signs one session of the account out. Another one: `204`, its
    /// sockets close. This app's own: a logout (`200 {"csrf_token"}`, the
    /// cookie cleared). `404` for one that is gone already.
    pub async fn revoke_session(&self, id: Uuid) -> Result<()> {
        let _: serde_json::Value = self
            .send_empty(Method::DELETE, &format!("/auth/sessions/{id}"))
            .await?;
        Ok(())
    }

    // --------------------------------------------------------------- me

    pub async fn me(&self) -> Result<User> {
        self.get("/me").await
    }

    pub async fn update_me(&self, patch: &ProfilePatch) -> Result<User> {
        self.send(Method::PATCH, "/me", patch).await
    }

    /// The account's language, `de` or `en` (v0.10 servers).
    pub async fn set_locale(&self, locale: &str) -> Result<LocaleBody> {
        let body = LocaleBody {
            locale: locale.to_owned(),
        };
        self.send(Method::PUT, "/me/locale", &body).await
    }

    /// Raw PNG/JPEG/WebP, at most 256×256 and 512 KiB.
    pub async fn upload_avatar(&self, content_type: &str, data: Vec<u8>) -> Result<User> {
        self.send_bytes(Method::PUT, "/me/avatar", content_type, data)
            .await
    }

    /// Raw PNG/JPEG/WebP, at most 960×320 and 1 MiB.
    pub async fn upload_banner(&self, content_type: &str, data: Vec<u8>) -> Result<User> {
        self.send_bytes(Method::PUT, "/me/banner", content_type, data)
            .await
    }

    pub async fn user_profile(&self, user: Uuid) -> Result<PublicProfile> {
        self.get(&format!("/users/{user}")).await
    }

    pub async fn themes(&self) -> Result<ThemeDocument> {
        self.get("/me/themes").await
    }

    /// Send the document with the revision last read; `409 theme_conflict`
    /// when another device saved meanwhile.
    pub async fn put_themes(&self, doc: &ThemeDocument) -> Result<ThemeDocument> {
        self.send(Method::PUT, "/me/themes", doc).await
    }

    /// `GET /api/me/export`.
    pub async fn export_status(&self) -> Result<ExportStatus> {
        self.get("/me/export").await
    }

    /// `POST /api/me/export` (202): a new archive, replacing a failed or
    /// day-old one. `409 export_pending` while one is being built,
    /// `429 rate_limited` (with `retry_after`) within a day of the last.
    pub async fn request_export(&self) -> Result<ExportStatus> {
        self.send_empty(Method::POST, "/me/export").await
    }

    /// `DELETE /api/me`. `202` ends the account and this session (the
    /// server clears the cookie); `409 owns_servers` names the servers in
    /// the way. Field codes as `password`/`email`: `required`, `wrong`.
    pub async fn delete_account(
        &self,
        confirmation: &DeleteConfirmation,
    ) -> Result<AccountDeletion> {
        let refusal = match self
            .send_keeping_refusal::<_, serde_json::Value>(Method::DELETE, "/me", confirmation)
            .await
        {
            Ok(_) => return Ok(AccountDeletion::Deleted),
            Err(refusal) => refusal,
        };
        let (error, body) = *refusal;
        if error.code() != "owns_servers" {
            return Err(error);
        }
        let servers = body
            .get("servers")
            .cloned()
            .and_then(|list| serde_json::from_value::<Lenient<OwnedServer>>(list).ok())
            .map(|Lenient(list)| list)
            .unwrap_or_default();
        Ok(AccountDeletion::OwnsServers(servers))
    }

    // ---------------------------------------------------------- servers

    pub async fn servers(&self) -> Result<Vec<Server>> {
        self.get("/servers").await.map(|Lenient(v)| v)
    }

    pub async fn server(&self, id: Uuid) -> Result<ServerDetail> {
        self.get(&format!("/servers/{id}")).await
    }

    pub async fn create_server(&self, name: &str) -> Result<ServerDetail> {
        self.send(Method::POST, "/servers", &json!({"name": name}))
            .await
    }

    pub async fn update_server(&self, id: Uuid, patch: &ServerPatch) -> Result<Server> {
        self.send(Method::PATCH, &format!("/servers/{id}"), patch)
            .await
    }

    /// Owner only.
    pub async fn delete_server(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/servers/{id}"))
            .await
    }

    pub async fn leave_server(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::POST, &format!("/servers/{id}/leave"))
            .await
    }

    pub async fn upload_server_icon(
        &self,
        id: Uuid,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Server> {
        self.send_bytes(
            Method::PUT,
            &format!("/servers/{id}/icon"),
            content_type,
            data,
        )
        .await
    }

    pub async fn remove_server_icon(&self, id: Uuid) -> Result<Server> {
        self.send_empty(Method::DELETE, &format!("/servers/{id}/icon"))
            .await
    }

    pub async fn kick(&self, server: Uuid, user: Uuid) -> Result<()> {
        self.send(
            Method::POST,
            &format!("/servers/{server}/kick"),
            &json!({"user_id": user}),
        )
        .await
    }

    pub async fn ban(&self, server: Uuid, user: Uuid) -> Result<()> {
        self.send(
            Method::POST,
            &format!("/servers/{server}/ban"),
            &json!({"user_id": user}),
        )
        .await
    }

    pub async fn bans(&self, server: Uuid) -> Result<Vec<Ban>> {
        self.get(&format!("/servers/{server}/bans"))
            .await
            .map(|Lenient(v)| v)
    }

    pub async fn unban(&self, server: Uuid, user: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/servers/{server}/bans/{user}"))
            .await
    }

    /// `POST /api/servers/{id}/transfer` (owner only): `user` becomes the
    /// owner; the answer is the list row as the caller now sees it
    /// (`role: member`).
    pub async fn transfer_server(&self, server: Uuid, user: Uuid) -> Result<Server> {
        self.send(
            Method::POST,
            &format!("/servers/{server}/transfer"),
            &json!({"user_id": user}),
        )
        .await
    }

    // ---------------------------------------------- categories, channels

    pub async fn create_category(&self, server: Uuid, name: &str) -> Result<Category> {
        self.send(
            Method::POST,
            &format!("/servers/{server}/categories"),
            &json!({"name": name}),
        )
        .await
    }

    pub async fn rename_category(&self, id: Uuid, name: &str) -> Result<Category> {
        self.send(
            Method::PATCH,
            &format!("/categories/{id}"),
            &json!({"name": name}),
        )
        .await
    }

    /// Its channels stay, uncategorised.
    pub async fn delete_category(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/categories/{id}"))
            .await
    }

    /// `kind` is text or voice; text names are normalised by the server.
    /// `role_ids` limits the new channel to those roles; `None` or `[]`:
    /// everyone.
    pub async fn create_channel(
        &self,
        server: Uuid,
        name: &str,
        kind: ChannelKind,
        category: Option<Uuid>,
        role_ids: Option<&[Uuid]>,
    ) -> Result<Channel> {
        let kind = match kind {
            ChannelKind::Voice => "voice",
            _ => "text",
        };
        // The web always sends `category_id`, "" for none.
        let category = category.map(|c| c.to_string()).unwrap_or_default();
        let mut body = json!({"name": name, "kind": kind, "category_id": category});
        if let Some(role_ids) = role_ids {
            body["role_ids"] = json!(role_ids);
        }
        self.send(Method::POST, &format!("/servers/{server}/channels"), &body)
            .await
    }

    pub async fn update_channel(&self, id: Uuid, patch: &ChannelPatch) -> Result<Channel> {
        self.send(Method::PATCH, &format!("/channels/{id}"), patch)
            .await
    }

    pub async fn delete_channel(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/channels/{id}"))
            .await
    }

    // ----------------------------------------------------------- invites

    pub async fn invites(&self, server: Uuid) -> Result<Vec<Invite>> {
        self.get(&format!("/servers/{server}/invites"))
            .await
            .map(|Lenient(v)| v)
    }

    /// `max_uses` 1..=10000, `expires_in_hours` 1..=8760; `None` = unlimited.
    pub async fn create_invite(
        &self,
        server: Uuid,
        max_uses: Option<u32>,
        expires_in_hours: Option<u32>,
    ) -> Result<Invite> {
        #[derive(Serialize)]
        struct Body {
            #[serde(skip_serializing_if = "Option::is_none")]
            max_uses: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            expires_in_hours: Option<u32>,
        }
        self.send(
            Method::POST,
            &format!("/servers/{server}/invites"),
            &Body {
                max_uses,
                expires_in_hours,
            },
        )
        .await
    }

    pub async fn invite_preview(&self, code: &str) -> Result<InvitePreview> {
        self.get(&format!("/invites/{}", encode_component(code.trim())))
            .await
    }

    pub async fn revoke_invite(&self, code: &str) -> Result<()> {
        self.send_empty(
            Method::DELETE,
            &format!("/invites/{}", encode_component(code.trim())),
        )
        .await
    }

    /// Also `200` when already a member.
    pub async fn join_invite(&self, code: &str) -> Result<Server> {
        self.send_empty(
            Method::POST,
            &format!("/invites/{}/join", encode_component(code.trim())),
        )
        .await
    }

    /// The link the web shares: `<origin>/invite/<code>`.
    pub fn invite_link(&self, code: &str) -> String {
        self.url(&format!("/invite/{code}"))
    }

    // --------------------------------------------------------------- DMs

    pub async fn dms(&self) -> Result<Vec<DirectMessage>> {
        self.get("/dms").await.map(|Lenient(v)| v)
    }

    pub async fn dm(&self, id: Uuid) -> Result<DirectMessage> {
        self.get(&format!("/dms/{id}")).await
    }

    /// Idempotent per pair.
    pub async fn open_dm(&self, user: Uuid) -> Result<DirectMessage> {
        self.send(Method::POST, "/dms", &json!({"user_id": user}))
            .await
    }

    // ---------------------------------------------------------- messages

    pub async fn messages(&self, channel: Uuid, q: &HistoryQuery) -> Result<MessagePage> {
        let qs = query(&[
            ("before", q.before.clone()),
            ("after", q.after.clone()),
            ("limit", q.limit.map(|l| l.to_string())),
        ]);
        self.get(&format!("/channels/{channel}/messages{qs}")).await
    }

    /// Up to 30 messages either side of `message` (jump to a reply/search hit).
    pub async fn message_context(&self, channel: Uuid, message: Uuid) -> Result<MessageContext> {
        self.get(&format!("/channels/{channel}/messages/{message}/context"))
            .await
    }

    /// Full-text search, newest hits first page; page on with `before`.
    pub async fn search_messages(
        &self,
        channel: Uuid,
        q: &str,
        before: Option<String>,
        limit: Option<u32>,
    ) -> Result<MessagePage> {
        let qs = query(&[
            ("q", Some(q.to_owned())),
            ("limit", limit.map(|l| l.to_string())),
            ("before", before),
        ]);
        self.get(&format!("/channels/{channel}/messages/search{qs}"))
            .await
    }

    pub async fn create_message(&self, channel: Uuid, body: &NewMessage) -> Result<Message> {
        self.send(Method::POST, &format!("/channels/{channel}/messages"), body)
            .await
    }

    pub async fn edit_message(&self, id: Uuid, content: &str) -> Result<Message> {
        self.send(
            Method::PATCH,
            &format!("/messages/{id}"),
            &json!({"content": content}),
        )
        .await
    }

    pub async fn delete_message(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/messages/{id}"))
            .await
    }

    /// Removes the link previews for good.
    pub async fn suppress_previews(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/messages/{id}/previews"))
            .await
    }

    pub async fn add_reaction(&self, id: Uuid, emoji: &str) -> Result<Message> {
        self.send_empty(
            Method::PUT,
            &format!("/messages/{id}/reactions/{}", encode_component(emoji)),
        )
        .await
    }

    pub async fn remove_reaction(&self, id: Uuid, emoji: &str) -> Result<Message> {
        self.send_empty(
            Method::DELETE,
            &format!("/messages/{id}/reactions/{}", encode_component(emoji)),
        )
        .await
    }

    // -------------------------------------------------------- read state

    pub async fn unread(&self) -> Result<Vec<ReadState>> {
        self.get("/messages/unread").await.map(|Lenient(v)| v)
    }

    /// Up to the newest message (`None`) or to `message`; forward only.
    pub async fn mark_read(&self, channel: Uuid, message: Option<Uuid>) -> Result<ReadState> {
        let body = match message {
            Some(id) => json!({"message_id": id}),
            None => json!({}),
        };
        self.send(Method::PUT, &format!("/channels/{channel}/read"), &body)
            .await
    }

    /// Only the rows that moved.
    pub async fn mark_server_read(&self, server: Uuid) -> Result<Vec<ReadState>> {
        self.send_empty(Method::PUT, &format!("/servers/{server}/read"))
            .await
            .map(|Lenient(v)| v)
    }

    pub async fn mark_all_read(&self, scope: ReadScope) -> Result<Vec<ReadState>> {
        self.send(Method::PUT, "/messages/read-all", &json!({"scope": scope}))
            .await
            .map(|Lenient(v)| v)
    }

    // --------------------------------------------- notification settings

    pub async fn notification_settings(&self) -> Result<Vec<NotificationSetting>> {
        self.get("/notification-settings").await.map(|Lenient(v)| v)
    }

    pub async fn set_server_notifications(
        &self,
        server: Uuid,
        body: &NotificationSettingBody,
    ) -> Result<NotificationSetting> {
        self.send(
            Method::PUT,
            &format!("/servers/{server}/notification-settings"),
            body,
        )
        .await
    }

    /// For a text channel or a DM.
    pub async fn set_channel_notifications(
        &self,
        channel: Uuid,
        body: &NotificationSettingBody,
    ) -> Result<NotificationSetting> {
        self.send(
            Method::PUT,
            &format!("/channels/{channel}/notification-settings"),
            body,
        )
        .await
    }

    pub async fn notification_privacy(&self) -> Result<NotificationPrivacy> {
        self.get("/notification-privacy").await
    }

    pub async fn set_notification_privacy(
        &self,
        hide_content: bool,
    ) -> Result<NotificationPrivacy> {
        self.send(
            Method::PUT,
            "/notification-privacy",
            &NotificationPrivacy { hide_content },
        )
        .await
    }

    // ------------------------------------------------------- attachments

    pub async fn presign_attachment(
        &self,
        channel: Uuid,
        req: &PresignRequest,
    ) -> Result<Presigned> {
        self.send(
            Method::POST,
            &format!("/channels/{channel}/attachments"),
            req,
        )
        .await
    }

    /// Presign, then PUT the bytes; the returned id goes into the message's
    /// `attachment_ids`, which binds it.
    pub async fn upload_attachment(
        &self,
        channel: Uuid,
        filename: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Attachment> {
        let presigned = self
            .presign_attachment(
                channel,
                &PresignRequest {
                    filename: filename.to_owned(),
                    content_type: content_type.to_owned(),
                    size: data.len() as i64,
                },
            )
            .await?;
        self.put_upload(&presigned.upload_url, &presigned.headers, data)
            .await?;
        Ok(presigned.attachment)
    }

    /// The inline URL of an attachment (`?download=1` forces a download).
    pub fn attachment_url(&self, id: Uuid, download: bool) -> String {
        if download {
            self.url(&format!("/api/attachments/{id}?download=1"))
        } else {
            self.url(&format!("/api/attachments/{id}"))
        }
    }

    // -------------------------------------------------------------- GIFs

    pub async fn gif_status(&self) -> Result<GifStatus> {
        self.get("/gifs").await
    }

    /// Page 1 omits `page`, like the web.
    pub async fn gifs_trending(&self, page: Option<u32>) -> Result<GifPage> {
        let qs = query(&[("page", page.filter(|p| *p > 1).map(|p| p.to_string()))]);
        self.get(&format!("/gifs/trending{qs}")).await
    }

    pub async fn gifs_search(&self, q: &str, page: Option<u32>) -> Result<GifPage> {
        let qs = query(&[
            ("q", Some(q.trim().to_owned())),
            ("page", page.filter(|p| *p > 1).map(|p| p.to_string())),
        ]);
        self.get(&format!("/gifs/search{qs}")).await
    }

    /// Klipy media go through the server's proxy.
    pub fn gif_media_url(&self, url: &str) -> String {
        self.url(&format!(
            "/api/gifs/media{}",
            query(&[("url", Some(url.to_owned()))])
        ))
    }

    // ------------------------------------------------------------- media

    /// Single-use ticket for `/media/ws` of a voice channel.
    pub async fn media_ticket(&self, channel: Uuid) -> Result<MediaTicket> {
        self.send_empty(Method::POST, &format!("/channels/{channel}/media-ticket"))
            .await
    }

    // ------------------------------------------------------------- roles

    /// Needs `manage_server`; the role lands after the others.
    pub async fn create_role(&self, server: Uuid, role: &NewRole) -> Result<ServerRole> {
        self.send(Method::POST, &format!("/servers/{server}/roles"), role)
            .await
    }

    pub async fn update_role(&self, id: Uuid, patch: &RolePatch) -> Result<ServerRole> {
        self.send(Method::PATCH, &format!("/roles/{id}"), patch)
            .await
    }

    /// Its assignments go too; channels limited to it stay limited.
    pub async fn delete_role(&self, id: Uuid) -> Result<()> {
        self.send_empty(Method::DELETE, &format!("/roles/{id}"))
            .await
    }

    /// Full replacement of a member's custom roles.
    pub async fn set_member_roles(
        &self,
        server: Uuid,
        user: Uuid,
        role_ids: &[Uuid],
    ) -> Result<MemberRoles> {
        self.send(
            Method::PUT,
            &format!("/servers/{server}/members/{user}/roles"),
            &json!({"role_ids": role_ids}),
        )
        .await
    }

    // --------------------------------------------------------- directory

    /// Whether the directory is on, with each language's listing count.
    pub async fn directory_status(&self) -> Result<DirectoryStatus> {
        self.get("/directory").await
    }

    /// One page of the search, as the web's `listingsPath`: an empty `q`
    /// or `language` and offset 0 are left out.
    pub async fn listings(
        &self,
        q: Option<&str>,
        language: Option<&str>,
        offset: u32,
    ) -> Result<ListingPage> {
        let qs = query(&[
            ("q", q.filter(|q| !q.is_empty()).map(str::to_owned)),
            (
                "language",
                language.filter(|l| !l.is_empty()).map(str::to_owned),
            ),
            ("offset", (offset > 0).then(|| offset.to_string())),
        ]);
        self.get(&format!("/directory/listings{qs}")).await
    }

    /// An open server: the caller becomes a member; the answer is the list
    /// row. Also `200` when already a member.
    pub async fn join_listing(&self, server: Uuid) -> Result<Server> {
        self.send_empty(Method::POST, &format!("/directory/listings/{server}/join"))
            .await
    }

    /// "Beitritt anfragen" with an optional message (≤ 300 characters).
    pub async fn request_join(&self, server: Uuid, message: &str) -> Result<RequestAnswer> {
        self.send(
            Method::POST,
            &format!("/directory/listings/{server}/request"),
            &json!({"message": message}),
        )
        .await
    }

    // ------------------------------------------------------ public pages

    /// `GET /public-pages.json`: not under `/api` (the web container serves
    /// it next to the pages) and a read, so no CSRF header. A 404, an
    /// unreachable server, invalid JSON or another version all mean that
    /// nothing is published (web `fetchPublicPages`); this never fails.
    pub async fn public_pages(&self) -> Result<PublicPages> {
        let Ok(res) = self
            .reqwest()
            .get(self.url("/public-pages.json"))
            .send()
            .await
        else {
            return Ok(PublicPages::default());
        };
        if !res.status().is_success() {
            return Ok(PublicPages::default());
        }
        let Ok(bytes) = res.bytes().await else {
            return Ok(PublicPages::default());
        };
        Ok(serde_json::from_slice::<serde_json::Value>(&bytes)
            .map(|json| PublicPages::parse(&json))
            .unwrap_or_default())
    }

    pub async fn withdraw_request(&self, server: Uuid) -> Result<()> {
        self.send_empty(
            Method::DELETE,
            &format!("/directory/listings/{server}/request"),
        )
        .await
    }

    /// Operator's report route, open to members and to whoever sees the
    /// listing.
    pub async fn report_listing(&self, server: Uuid, body: &ReportBody) -> Result<ReportFiled> {
        self.send(Method::POST, &format!("/servers/{server}/reports"), body)
            .await
    }

    pub async fn listing_settings(&self, server: Uuid) -> Result<ListingSettings> {
        self.get(&format!("/servers/{server}/listing")).await
    }

    /// Only the owner may save.
    pub async fn put_listing_settings(
        &self,
        server: Uuid,
        body: &ListingSettingsBody,
    ) -> Result<ListingSettings> {
        self.send(Method::PUT, &format!("/servers/{server}/listing"), body)
            .await
    }

    /// The pending requests, oldest first (owner or `moderate_members`).
    pub async fn join_requests(&self, server: Uuid) -> Result<Vec<JoinRequest>> {
        self.get(&format!("/servers/{server}/join-requests"))
            .await
            .map(|Lenient(v)| v)
    }

    pub async fn accept_join_request(&self, server: Uuid, user: Uuid) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/servers/{server}/join-requests/{user}/accept"),
        )
        .await
    }

    pub async fn decline_join_request(&self, server: Uuid, user: Uuid) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/servers/{server}/join-requests/{user}/decline"),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;

    /// Answers every connection with `reply` and keeps the request heads.
    fn serve(reply: &'static str) -> (String, std::sync::Arc<parking_lot::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                log.lock()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn fetch(base: &str) -> PublicPages {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let http = Http::new(base).unwrap();
        runtime.block_on(http.public_pages()).unwrap()
    }

    #[test]
    fn public_pages_come_from_the_instance_root() {
        let body = r#"{"version":1,"landing":true,"legal":{"privacy":{"de":"/de/datenschutz","en":"/en/privacy"}}}"#;
        let reply: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        );
        let (base, seen) = serve(reply);
        let pages = fetch(&base);
        assert!(pages.landing);
        assert_eq!(pages.legal.len(), 1);
        assert_eq!(pages.legal[0].0, LegalPageKind::Privacy);
        let head = seen.lock()[0].clone();
        assert!(head.starts_with("GET /public-pages.json "), "{head}");
        assert!(
            !head.to_ascii_lowercase().contains("x-csrf-token"),
            "{head}"
        );
    }

    #[test]
    fn public_pages_fall_back_to_nothing() {
        let (missing, _) =
            serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert_eq!(fetch(&missing), PublicPages::default());
        let (broken, _) = serve(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 9\r\nConnection: close\r\n\r\n<!doctype",
        );
        assert_eq!(fetch(&broken), PublicPages::default());
        // Nobody listens there.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        assert_eq!(fetch(&closed), PublicPages::default());
    }

    /// One request as the fake API saw it: request line and body.
    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// A fake API: the session route hands out a CSRF token, `answer`
    /// gives status and body for the rest by request line.
    fn fake_api(answer: fn(&str) -> (&'static str, String)) -> (String, Seen) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    // The head, then as much body as it announces.
                    let (head, body) = loop {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        data.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&data).into_owned();
                        let Some((head, body)) = text.split_once("\r\n\r\n") else {
                            continue;
                        };
                        let length = head
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break (head.to_owned(), body.to_owned());
                        }
                    };
                    let line = head.lines().next().unwrap_or("").to_owned();
                    let (status, reply) = if line.starts_with("GET /api/auth/session ") {
                        ("200 OK", r#"{"user":null,"csrf_token":"t"}"#.to_owned())
                    } else {
                        log.lock().push((line.clone(), body));
                        answer(&line)
                    };
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                            reply.len()
                        )
                        .as_bytes(),
                    );
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn server_row(id: Uuid) -> String {
        serde_json::json!({
            "id": id, "name": "Filmclub", "owner_id": Uuid::from_u128(9), "icon_url": null,
            "created_at": "2026-10-01T00:00:00Z", "role": "member",
            "permissions": [], "member_permissions": [],
        })
        .to_string()
    }

    /// The search's query, as the web's `listingsPath` writes it.
    #[test]
    fn the_search_leaves_out_what_is_not_set() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (base, seen) = fake_api(|line| {
            let body = if line.starts_with("GET /api/directory ") {
                r#"{"enabled":true,"languages":[{"code":"de","name":"Deutsch","listings":3}]}"#
            } else {
                r#"{"listings":[],"next_offset":null}"#
            };
            ("200 OK", body.to_owned())
        });
        let http = Http::new(&base).unwrap();
        let status = runtime.block_on(http.directory_status()).unwrap();
        assert_eq!(status.languages[0].listings, 3);
        for (q, language, offset) in [
            (None, None, 0),
            (Some(""), Some(""), 0),
            (Some("rust & co"), None, 0),
            (None, Some("en"), 30),
            (Some("50%_"), Some("de"), 60),
        ] {
            let page = runtime
                .block_on(http.listings(q, language, offset))
                .unwrap();
            assert!(page.listings.is_empty() && page.next_offset.is_none());
        }
        let lines: Vec<String> = seen.lock().iter().map(|(l, _)| l.clone()).collect();
        assert_eq!(
            lines,
            [
                "GET /api/directory HTTP/1.1",
                "GET /api/directory/listings HTTP/1.1",
                "GET /api/directory/listings HTTP/1.1",
                "GET /api/directory/listings?q=rust+%26+co HTTP/1.1",
                "GET /api/directory/listings?language=en&offset=30 HTTP/1.1",
                "GET /api/directory/listings?q=50%25_&language=de&offset=60 HTTP/1.1",
            ]
        );
    }

    /// Every write goes to its route with the web's body.
    #[test]
    fn the_writes_go_to_their_routes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (base, seen) = fake_api(|line| {
            let server = Uuid::from_u128(1);
            match line.split(' ').take(2).collect::<Vec<_>>().as_slice() {
                [_, path] if path.ends_with("/join") => ("200 OK", server_row(server)),
                ["POST", path] if path.ends_with("/request") => {
                    ("201 Created", r#"{"request":"pending"}"#.to_owned())
                }
                ["POST", path] if path.ends_with("/reports") => (
                    "201 Created",
                    r#"{"id":"00000000-0000-0000-0000-000000000004","created_at":"2026-11-10T10:00:00Z"}"#
                        .to_owned(),
                ),
                [_, path] if path.ends_with("/listing") => (
                    "200 OK",
                    r#"{"listed":true,"description":"Filme.","language":"de","join_mode":"open","delisted":false,"listed_at":"2026-11-10T10:00:00Z"}"#
                        .to_owned(),
                ),
                ["GET", path] if path.ends_with("/join-requests") => (
                    "200 OK",
                    r#"[{"user_id":"00000000-0000-0000-0000-000000000003","name":"Kai","avatar_url":null,"message":"","created_at":"2026-11-10T10:00:00Z"}]"#
                        .to_owned(),
                ),
                _ => ("204 No Content", String::new()),
            }
        });
        let http = Http::new(&base).unwrap();
        let (server, kai) = (Uuid::from_u128(1), Uuid::from_u128(3));
        runtime.block_on(async {
            assert_eq!(http.join_listing(server).await.unwrap().id, server);
            let answer = http.request_join(server, "Hallo!").await.unwrap();
            assert_eq!(answer.request, RequestState::Pending);
            http.withdraw_request(server).await.unwrap();
            let report = ReportBody {
                reason: ReportReason::Other,
                details: Some("Werbung".into()),
            };
            assert_eq!(
                http.report_listing(server, &report).await.unwrap().id,
                Uuid::from_u128(4)
            );
            assert!(http.listing_settings(server).await.unwrap().listed);
            let form = ListingSettingsBody {
                listed: true,
                description: "Filme.".into(),
                language: "de".into(),
                join_mode: JoinMode::Open,
            };
            let saved = http.put_listing_settings(server, &form).await.unwrap();
            assert_eq!(saved.join_mode, JoinMode::Open);
            assert_eq!(http.join_requests(server).await.unwrap()[0].user_id, kai);
            http.accept_join_request(server, kai).await.unwrap();
            http.decline_join_request(server, kai).await.unwrap();
        });
        let s = "00000000-0000-0000-0000-000000000001";
        let u = "00000000-0000-0000-0000-000000000003";
        let expected = [
            (format!("POST /api/directory/listings/{s}/join"), ""),
            (
                format!("POST /api/directory/listings/{s}/request"),
                r#"{"message":"Hallo!"}"#,
            ),
            (format!("DELETE /api/directory/listings/{s}/request"), ""),
            (
                format!("POST /api/servers/{s}/reports"),
                r#"{"reason":"other","details":"Werbung"}"#,
            ),
            (format!("GET /api/servers/{s}/listing"), ""),
            (
                format!("PUT /api/servers/{s}/listing"),
                r#"{"listed":true,"description":"Filme.","language":"de","join_mode":"open"}"#,
            ),
            (format!("GET /api/servers/{s}/join-requests"), ""),
            (
                format!("POST /api/servers/{s}/join-requests/{u}/accept"),
                "",
            ),
            (
                format!("POST /api/servers/{s}/join-requests/{u}/decline"),
                "",
            ),
        ];
        let seen = seen.lock();
        assert_eq!(seen.len(), expected.len(), "{seen:?}");
        for ((line, body), (want_line, want_body)) in seen.iter().zip(expected) {
            assert_eq!(line, &format!("{want_line} HTTP/1.1"));
            assert_eq!(body, want_body, "{line}");
        }
    }

    /// The directory's refusals keep their codes and copy.
    #[test]
    fn the_directory_codes_are_known() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (base, _) = fake_api(|_| {
            (
                "409 Conflict",
                r#"{"error":"join_request_declined","message":"declined","retry_after":3600}"#
                    .to_owned(),
            )
        });
        let http = Http::new(&base).unwrap();
        let error = runtime
            .block_on(http.request_join(Uuid::from_u128(1), ""))
            .unwrap_err();
        assert_eq!(error.code(), "join_request_declined");
        assert_eq!(error.retry_after(), Some(3600.0));
        let _german = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::De);
        for (code, text) in [
            (
                "join_request_required",
                "Diesem Server trittst du nur auf Anfrage bei.",
            ),
            (
                "join_request_declined",
                "Deine Anfrage wurde nicht angenommen. Du kannst in ein paar Tagen wieder anfragen.",
            ),
            (
                "join_requests_full",
                "Gerade sind zu viele Anfragen offen. Bitte später noch einmal versuchen.",
            ),
            ("already_member", "Du bist schon Mitglied dieses Servers."),
        ] {
            assert_eq!(crate::http::error_message(code), text);
        }
        drop(_german);
        let _english = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
        assert_eq!(
            crate::http::error_message("already_member"),
            "You are already a member of this server."
        );
    }

    /// Devices, export, deletion and transfer go to their routes with the
    /// web's bodies (#519).
    #[test]
    fn the_account_calls_go_to_their_routes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (base, seen) = fake_api(|line| {
            let export = r#"{"export":{"state":"pending","requested_at":"2026-11-11T10:00:00Z","finished_at":null,"expires_at":null,"size_bytes":null},"mail":true}"#;
            match line.split(' ').take(2).collect::<Vec<_>>().as_slice() {
                ["GET", "/api/auth/sessions"] => (
                    "200 OK",
                    r#"{"sessions":[{"id":"00000000-0000-0000-0000-000000000001","client":"desktop","os":"windows","created_at":"2026-11-01T18:00:00Z","last_used_at":"2026-11-10T19:02:00Z","current":true},{"id":"x"}]}"#
                        .to_owned(),
                ),
                ["DELETE", path] if path.starts_with("/api/auth/sessions/") => {
                    ("204 No Content", String::new())
                }
                ["GET", "/api/me/export"] => ("200 OK", r#"{"export":null,"mail":false}"#.to_owned()),
                ["POST", "/api/me/export"] => ("202 Accepted", export.to_owned()),
                ["DELETE", "/api/me"] => ("202 Accepted", r#"{"csrf_token":"n"}"#.to_owned()),
                ["POST", path] if path.ends_with("/transfer") => {
                    ("200 OK", server_row(Uuid::from_u128(1)))
                }
                _ => ("500 Internal Server Error", String::new()),
            }
        });
        let http = Http::new(&base).unwrap();
        let (server, kai) = (Uuid::from_u128(1), Uuid::from_u128(3));
        runtime.block_on(async {
            let sessions = http.sessions().await.unwrap();
            assert_eq!(sessions.len(), 1, "the unreadable row is left out");
            assert_eq!(sessions[0].os, DeviceOs::Windows);
            http.revoke_session(Uuid::from_u128(2)).await.unwrap();
            assert!(http.export_status().await.unwrap().export.is_none());
            assert!(http.request_export().await.unwrap().building());
            let deleted = http
                .delete_account(&DeleteConfirmation::Password("pw".into()))
                .await
                .unwrap();
            assert_eq!(deleted, AccountDeletion::Deleted);
            assert_eq!(http.csrf_token().as_deref(), Some("n"));
            let row = http.transfer_server(server, kai).await.unwrap();
            assert_eq!(row.id, server);
        });
        let s = "00000000-0000-0000-0000-000000000001";
        let expected = [
            ("GET /api/auth/sessions".to_owned(), String::new()),
            (
                "DELETE /api/auth/sessions/00000000-0000-0000-0000-000000000002".to_owned(),
                String::new(),
            ),
            ("GET /api/me/export".to_owned(), String::new()),
            ("POST /api/me/export".to_owned(), String::new()),
            (
                "DELETE /api/me".to_owned(),
                r#"{"password":"pw"}"#.to_owned(),
            ),
            (
                format!("POST /api/servers/{s}/transfer"),
                r#"{"user_id":"00000000-0000-0000-0000-000000000003"}"#.to_owned(),
            ),
        ];
        let seen = seen.lock();
        assert_eq!(seen.len(), expected.len(), "{seen:?}");
        for ((line, body), (want_line, want_body)) in seen.iter().zip(expected) {
            assert_eq!(line, &format!("{want_line} HTTP/1.1"));
            assert_eq!(body, &want_body, "{line}");
        }
    }

    /// `409 owns_servers` is an answer, not a failure: it names the servers
    /// in the way. Other refusals stay errors with their field codes.
    #[test]
    fn a_deletion_names_the_servers_in_the_way() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (base, _) = fake_api(|_| {
            (
                "409 Conflict",
                r#"{"error":"owns_servers","message":"Transfer or delete your servers first.","servers":[{"id":"00000000-0000-0000-0000-000000000007","name":"Wohnzimmer","member_count":5}]}"#
                    .to_owned(),
            )
        });
        let http = Http::new(&base).unwrap();
        let answer = runtime
            .block_on(http.delete_account(&DeleteConfirmation::Email("lena@example.test".into())))
            .unwrap();
        assert_eq!(
            answer,
            AccountDeletion::OwnsServers(vec![OwnedServer {
                id: Uuid::from_u128(7),
                name: "Wohnzimmer".into(),
                member_count: Some(5),
            }])
        );
        let (base, _) = fake_api(|_| {
            (
                "422 Unprocessable Entity",
                r#"{"error":"validation_failed","message":"x","fields":{"password":"wrong"}}"#
                    .to_owned(),
            )
        });
        let http = Http::new(&base).unwrap();
        let error = runtime
            .block_on(http.delete_account(&DeleteConfirmation::Password("nope".into())))
            .unwrap_err();
        assert_eq!(error.field("password"), Some("wrong"));
        let (base, _) = fake_api(|_| {
            (
                "409 Conflict",
                r#"{"error":"export_pending","message":"x"}"#.to_owned(),
            )
        });
        let http = Http::new(&base).unwrap();
        let error = runtime.block_on(http.request_export()).unwrap_err();
        assert_eq!(error.code(), "export_pending");
    }
}
