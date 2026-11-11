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
        if self.csrf_token().is_none() {
            self.session().await?;
        }
        self.send(
            Method::POST,
            "/auth/register",
            // The language the app shows becomes the account's.
            &json!({
                "email": email,
                "password": password,
                "name": name,
                "locale": gelabber_i18n::get().as_str(),
            }),
        )
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers every connection with `reply` and keeps the request heads.
    fn serve(reply: &'static str) -> (String, std::sync::Arc<parking_lot::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
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
}
