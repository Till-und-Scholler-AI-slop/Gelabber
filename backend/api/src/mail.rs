//! Outgoing account mail (#195): address confirmation, password reset, the
//! notice for a registration with a known address, beta invitations
//! (#517), the notice after an account was deleted and the one when a data
//! export is ready (#519), and the mails around feedback (#542).
//!
//! Mail goes to the operator's SMTP relay (`SMTP_*`), plain text, in the
//! account's language (`crate::i18n::mail` holds the texts). Sending runs in
//! the background after the response (`crate::auth`), so neither the relay's
//! speed nor its failures tell a caller whether an address has an account.
//! Failures are logged with the user id, never the address.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lettre::Address;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::{MailConfig, SmtpSecurity};
use crate::feedback::Kind;
use crate::i18n::{self, Locale};

/// Per connection step; a hanging relay must not pile up tasks.
const SMTP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct Mailer {
    from: Mailbox,
    public_url: String,
    instance: String,
    transport: Transport,
}

#[derive(Clone)]
enum Transport {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    /// Tests read what would have been sent.
    Memory(Arc<Mutex<Vec<Sent>>>),
}

/// A mail as the memory transport keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// What to tell the recipient. Tokens are the raw values from
/// [`crate::auth::tokens::issue`]; only their hashes are stored.
#[derive(Debug, Clone)]
pub enum Mail {
    ConfirmEmail {
        token: String,
    },
    ResetPassword {
        token: String,
    },
    /// Someone registered with an address that already has an account.
    AlreadyRegistered,
    /// An operator invited this address to register (#517); `link` is the
    /// invitation's registration link.
    BetaInvitation {
        link: String,
    },
    /// A provider account was connected to this account (#518), by the
    /// owner or by a sign-in that joined it; `provider_name` is "Google".
    ProviderConnected {
        provider_name: String,
    },
    /// The account was deleted and its data is gone (#519); sent to the
    /// address it had, in the language it had.
    AccountDeleted,
    /// The account's data export is ready to download until `expires_at`
    /// (#519). It links to the settings page and carries no token: the
    /// download needs the account's session.
    ExportReady {
        expires_at: chrono::DateTime<chrono::Utc>,
    },
    /// A visitor sent the public feedback form with this address (#542);
    /// `link` confirms the entry. It never quotes the visitor's text.
    FeedbackConfirm {
        link: String,
    },
    /// To an operator: a new feedback entry, or with `reply` a new reply
    /// from its sender (#542). `sender` is `Name (address)` or a visitor's
    /// address, `excerpt` the start of the text, `link` the entry in
    /// Betrieb.
    FeedbackNotice {
        kind: Kind,
        reply: bool,
        sender: String,
        excerpt: String,
        link: String,
    },
    /// To the sender's account: the operators replied (#542). Without the
    /// reply; it links to the settings.
    FeedbackAnswered,
    /// To a visitor: an operator's reply `text` to the entry of `kind` they
    /// sent on `sent_on` (#542).
    FeedbackReply {
        kind: Kind,
        sent_on: chrono::DateTime<chrono::Utc>,
        text: String,
    },
}

impl Mailer {
    pub fn new(config: &MailConfig) -> Result<Self, String> {
        let from: Mailbox = config
            .from
            .parse()
            .map_err(|err| format!("MAIL_FROM: {err}"))?;
        let builder = match config.security {
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            }
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
            SmtpSecurity::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &config.host,
            )),
        }
        .map_err(|err| format!("SMTP_HOST: {err}"))?;
        // EHLO with the instance's own name rather than the container's.
        let hello = config
            .public_url
            .host_str()
            .map(|host| ClientId::Domain(host.to_owned()))
            .unwrap_or_default();
        let mut builder = builder
            .port(config.port)
            .timeout(Some(SMTP_TIMEOUT))
            .hello_name(hello);
        if let Some((user, password)) = &config.credentials {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self::with_transport(
            from,
            &config.public_url,
            Transport::Smtp(builder.build()),
        ))
    }

    /// A mailer that keeps every mail in memory; see [`Self::outbox`].
    pub fn memory(public_url: &url::Url) -> Self {
        let from = "Gelabber <noreply@gelabber.test>"
            .parse()
            .expect("static mailbox");
        Self::with_transport(from, public_url, Transport::Memory(Arc::default()))
    }

    fn with_transport(from: Mailbox, public_url: &url::Url, transport: Transport) -> Self {
        Self {
            from,
            public_url: public_url.as_str().trim_end_matches('/').to_owned(),
            instance: public_url.host_str().unwrap_or("Gelabber").to_owned(),
            transport,
        }
    }

    /// The public URL links in mails start with, without a trailing slash.
    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    /// Mails the memory transport took so far (empty for SMTP).
    pub fn outbox(&self) -> Vec<Sent> {
        match &self.transport {
            Transport::Memory(sent) => sent.lock().unwrap_or_else(|err| err.into_inner()).clone(),
            Transport::Smtp(_) => Vec::new(),
        }
    }

    pub async fn send(&self, to: &str, mail: &Mail, locale: Locale) -> Result<(), String> {
        let (subject, body) = self.render(mail, locale);
        match &self.transport {
            Transport::Memory(sent) => {
                sent.lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(Sent {
                        to: to.to_owned(),
                        subject,
                        body,
                    });
                Ok(())
            }
            Transport::Smtp(transport) => {
                let to = recipient(to)?;
                let message = Message::builder()
                    .from(self.from.clone())
                    .to(to)
                    .subject(subject)
                    .header(ContentType::TEXT_PLAIN)
                    .body(body)
                    .map_err(|err| format!("message: {err}"))?;
                transport
                    .send(message)
                    .await
                    .map(|_| ())
                    .map_err(|err| format!("smtp: {err}"))
            }
        }
    }

    /// Subject and body in `locale` ([`i18n::mail::render`]).
    pub fn render(&self, mail: &Mail, locale: Locale) -> (String, String) {
        i18n::mail::render(locale, mail, &self.public_url, &self.instance)
    }
}

/// The stored address as a bare mailbox. Parsing it as `Name <addr>` would
/// deliver an older account's `a<victim@example.org>` to victim@example.org.
fn recipient(to: &str) -> Result<Mailbox, String> {
    let address: Address = to.parse().map_err(|err| format!("recipient: {err}"))?;
    Ok(Mailbox::new(None, address))
}

impl Mail {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ConfirmEmail { .. } => "confirm_email",
            Self::ResetPassword { .. } => "reset_password",
            Self::AlreadyRegistered => "already_registered",
            Self::BetaInvitation { .. } => "beta_invitation",
            Self::ProviderConnected { .. } => "provider_connected",
            Self::AccountDeleted => "account_deleted",
            Self::ExportReady { .. } => "export_ready",
            Self::FeedbackConfirm { .. } => "feedback_confirm",
            Self::FeedbackNotice { .. } => "feedback_notice",
            Self::FeedbackAnswered => "feedback_answered",
            Self::FeedbackReply { .. } => "feedback_reply",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailer() -> Mailer {
        Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap())
    }

    #[tokio::test]
    async fn links_point_at_the_public_url_with_the_token_in_the_fragment() {
        let mailer = mailer();
        mailer
            .send(
                "ada@example.org",
                &Mail::ResetPassword {
                    token: "abc".into(),
                },
                Locale::De,
            )
            .await
            .unwrap();
        let sent = mailer.outbox();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].to, "ada@example.org");
        assert!(
            sent[0]
                .body
                .contains("https://chat.example.org/reset-password#token=abc"),
            "{}",
            sent[0].body
        );
        assert!(sent[0].body.contains("chat.example.org"));
    }

    #[test]
    fn feedback_mails_have_their_kinds() {
        let now = chrono::Utc::now();
        let mails = [
            Mail::FeedbackConfirm { link: "x".into() },
            Mail::FeedbackNotice {
                kind: Kind::Idea,
                reply: false,
                sender: "x".into(),
                excerpt: "x".into(),
                link: "x".into(),
            },
            Mail::FeedbackAnswered,
            Mail::FeedbackReply {
                kind: Kind::Idea,
                sent_on: now,
                text: "x".into(),
            },
        ];
        let kinds: Vec<&str> = mails.iter().map(Mail::kind).collect();
        assert_eq!(
            kinds,
            [
                "feedback_confirm",
                "feedback_notice",
                "feedback_answered",
                "feedback_reply"
            ]
        );
    }

    #[test]
    fn recipients_are_bare_addresses() {
        let to = recipient("ada@example.org").unwrap();
        assert_eq!(to.name, None);
        assert_eq!(to.email.to_string(), "ada@example.org");
        for stored in ["a<victim@example.org>", "<victim@example.org>"] {
            assert!(recipient(stored).is_err(), "{stored}");
        }
    }

    #[test]
    fn smtp_transport_builds_for_every_security_mode() {
        for security in [
            SmtpSecurity::StartTls,
            SmtpSecurity::Tls,
            SmtpSecurity::None,
        ] {
            let config = MailConfig {
                host: "smtp.example.org".into(),
                port: security.default_port(),
                security,
                credentials: None,
                from: "Gelabber <noreply@example.org>".into(),
                public_url: url::Url::parse("https://chat.example.org").unwrap(),
            };
            assert!(Mailer::new(&config).is_ok(), "{security:?}");
        }
    }
}
