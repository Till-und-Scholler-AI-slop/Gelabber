//! Outgoing account mail (#195): address confirmation, password reset and
//! the notice for a registration with a known address.
//!
//! Mail goes to the operator's SMTP relay (`SMTP_*`), plain text, German like
//! the rest of the product. Sending runs in the background after the
//! response, so neither the relay's speed nor its failures tell a caller
//! whether an address has an account. Failures are logged with the user id,
//! never the address.
//!
//! The texts never contain what a registrant typed (name, password): a
//! confirmation mail can reach someone who never asked for it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::{MailConfig, SmtpSecurity};

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

    /// Mails the memory transport took so far (empty for SMTP).
    pub fn outbox(&self) -> Vec<Sent> {
        match &self.transport {
            Transport::Memory(sent) => sent.lock().unwrap_or_else(|err| err.into_inner()).clone(),
            Transport::Smtp(_) => Vec::new(),
        }
    }

    /// Sends in the background; the caller answers without waiting.
    pub fn spawn(&self, user_id: Uuid, to: &str, mail: Mail) {
        let mailer = self.clone();
        let to = to.to_owned();
        tokio::spawn(async move {
            let kind = mail.kind();
            match mailer.send(&to, &mail).await {
                Ok(()) => info!(%user_id, kind, "account mail sent"),
                Err(err) => warn!(%user_id, kind, error = %err, "account mail failed"),
            }
        });
    }

    pub async fn send(&self, to: &str, mail: &Mail) -> Result<(), String> {
        let (subject, body) = self.render(mail);
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
                let to: Mailbox = to.parse().map_err(|err| format!("recipient: {err}"))?;
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

    fn render(&self, mail: &Mail) -> (String, String) {
        let base = &self.public_url;
        let instance = &self.instance;
        match mail {
            Mail::ConfirmEmail { token } => (
                "Bestätige deine E-Mail-Adresse für Gelabber".to_owned(),
                format!(
                    "Hallo,\n\n\
                     mit dieser Adresse wurde auf {instance} ein Gelabber-Konto angelegt. \
                     Öffne diesen Link, um die Adresse zu bestätigen:\n\n\
                     {base}/verify-email#token={token}\n\n\
                     Der Link gilt 24 Stunden und nur einmal.\n\n\
                     Warst du das nicht? Dann ignoriere diese Mail. Ohne Bestätigung \
                     lässt sich das Konto nicht benutzen.\n"
                ),
            ),
            Mail::ResetPassword { token } => (
                "Neues Passwort für Gelabber".to_owned(),
                format!(
                    "Hallo,\n\n\
                     für dein Gelabber-Konto auf {instance} wurde ein neues Passwort \
                     angefordert. Hier kannst du es festlegen:\n\n\
                     {base}/reset-password#token={token}\n\n\
                     Der Link gilt eine Stunde und nur einmal. Danach bist du auf \
                     allen Geräten abgemeldet.\n\n\
                     Hast du nichts angefordert? Dann ignoriere diese Mail; dein \
                     Passwort bleibt, wie es ist.\n"
                ),
            ),
            Mail::AlreadyRegistered => (
                "Dein Gelabber-Konto".to_owned(),
                format!(
                    "Hallo,\n\n\
                     gerade wollte jemand auf {instance} ein Gelabber-Konto mit dieser \
                     Adresse anlegen. Du hast dort schon eins.\n\n\
                     Anmelden: {base}/login\n\
                     Passwort vergessen: {base}/forgot-password\n\n\
                     Warst du das nicht? Dann ignoriere diese Mail; an deinem Konto \
                     hat sich nichts geändert.\n"
                ),
            ),
        }
    }
}

impl Mail {
    fn kind(&self) -> &'static str {
        match self {
            Self::ConfirmEmail { .. } => "confirm_email",
            Self::ResetPassword { .. } => "reset_password",
            Self::AlreadyRegistered => "already_registered",
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
