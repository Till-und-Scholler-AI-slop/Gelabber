//! Outbound requests for link previews, guarded against SSRF. A message can
//! name any URL, so every request, redirects included, is checked here
//! first (scheme, port, no credentials, no local names), and the one client
//! resolves names only through [`Fetcher`]'s resolver, which refuses a name
//! unless every address is public. That answer is the one connected to, so
//! a second DNS answer cannot move the connection. Only ports 80 and 443,
//! no cookies, no proxy, small bodies and short timeouts; a host that let a
//! request time out is skipped for a while.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use tokio::sync::Semaphore;
use url::{Host, Url};

/// Each request, connect to last body byte.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
/// Name lookups at once. A lookup that hangs keeps its blocking thread
/// until the system resolver gives up; this bounds how many can.
const DNS_LOOKUPS: usize = 8;
/// A host or address that let a request time out is skipped this long, so
/// a slow site cannot keep taking the workers.
pub const BACKOFF: Duration = Duration::from_secs(600);
const BACKOFF_ENTRIES: usize = 4096;
pub const MAX_REDIRECTS: usize = 3;
/// Pages: only the head matters, so a longer page is cut, not refused.
pub const HTML_MAX_BYTES: usize = 1024 * 1024;
pub const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const JSON_MAX_BYTES: usize = 256 * 1024;
/// "like Twitterbot", as Telegram's preview bot says it: sites such as
/// TikTok serve their OpenGraph tags only to preview bots they know and
/// give anyone else an empty app shell (#349).
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; GelabberBot/",
    env!("CARGO_PKG_VERSION"),
    "; +https://github.com/Till-und-Scholler-AI-slop/Gelabber; like Twitterbot)"
);

/// Host names that are never fetched, whatever they resolve to: single
/// labels (Compose service names like `postgres`, `minio`, `localhost`)
/// and the suffixes reserved for local networks.
const LOCAL_SUFFIXES: &[&str] = &[
    ".localhost",
    ".local",
    ".internal",
    ".lan",
    ".home",
    ".home.arpa",
    ".intranet",
    ".corp",
    ".private",
];

#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// The deployment's own hosts (database, Redis, MinIO, TURN) and what
    /// the operator adds; a name matches itself and its subdomains.
    pub deny_hosts: Vec<String>,
    /// Tests only: fixed DNS answers, consulted before the resolver.
    pub hosts: HashMap<String, Vec<IpAddr>>,
    /// Tests only: socket addresses treated as public, and their ports
    /// allowed. Empty in production, where there is no way to set it.
    pub trusted: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The URL or an address it resolves to is not allowed.
    Refused(&'static str),
    Dns,
    Timeout,
    Http(String),
    Status(u16),
    TooManyRedirects,
    ContentType,
    TooLarge,
    /// The host timed out recently ([`BACKOFF`]).
    Backoff,
}

impl FetchError {
    /// A short, fixed reason for logs and the metric label.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Refused(_) => "refused",
            Self::Dns => "dns",
            Self::Timeout => "timeout",
            Self::Http(_) => "http",
            Self::Status(_) => "status",
            Self::TooManyRedirects => "redirects",
            Self::ContentType => "content_type",
            Self::TooLarge => "too_large",
            Self::Backoff => "backoff",
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for FetchError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Html,
    Image,
    Json,
}

impl Kind {
    fn accept(self) -> &'static str {
        match self {
            Self::Html => "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1",
            Self::Image => "image/webp,image/png,image/jpeg;q=0.9",
            Self::Json => "application/json",
        }
    }

    fn max_bytes(self) -> usize {
        match self {
            Self::Html => HTML_MAX_BYTES,
            Self::Image => IMAGE_MAX_BYTES,
            Self::Json => JSON_MAX_BYTES,
        }
    }

    fn allows(self, content_type: &str) -> bool {
        match self {
            Self::Html => matches!(content_type, "text/html" | "application/xhtml+xml"),
            Self::Image => matches!(content_type, "image/png" | "image/jpeg" | "image/webp"),
            Self::Json => content_type == "application/json" || content_type.ends_with("+json"),
        }
    }
}

#[derive(Debug)]
pub struct Fetched {
    /// Where the body came from, after redirects.
    pub url: Url,
    /// Lowercase media type without parameters.
    pub content_type: String,
    /// The `charset` parameter, lowercase, if any.
    pub charset: Option<String>,
    pub body: Vec<u8>,
}

/// The previews' HTTP client: one connection pool, names resolved only
/// through the guard.
#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    guard: Arc<Guard>,
}

struct Guard {
    policy: Policy,
    lookups: Arc<Semaphore>,
    /// Hosts in backoff, until when. Only the host that timed out: its
    /// addresses may be a CDN's, shared with much of the web.
    slow: Mutex<HashMap<String, Instant>>,
}

/// The client's DNS resolver: [`Guard::lookup`], so every connection goes
/// to an address that was vetted for it.
struct Vetting(Arc<Guard>);

impl Resolve for Vetting {
    fn resolve(&self, name: Name) -> Resolving {
        let guard = self.0.clone();
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let ips = guard.lookup(&name).await?;
            // Port 0: the client puts in the URL's port.
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

impl Fetcher {
    pub fn new(policy: Policy) -> Self {
        let guard = Arc::new(Guard {
            policy,
            lookups: Arc::new(Semaphore::new(DNS_LOOKUPS)),
            slow: Mutex::default(),
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2)
            .user_agent(USER_AGENT)
            .dns_resolver(Vetting(guard.clone()))
            .build()
            .expect("link preview HTTP client");
        Self { client, guard }
    }

    pub fn policy(&self) -> &Policy {
        &self.guard.policy
    }

    /// GET `url` as `kind`, following up to [`MAX_REDIRECTS`] redirects,
    /// each checked like the first request.
    pub async fn get(&self, url: &Url, kind: Kind) -> Result<Fetched, FetchError> {
        let mut url = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            self.guard.policy.check_url(&url)?;
            if let Some(ip) = literal_ip(&url)
                && self.guard.in_backoff(&ip.to_string())
            {
                return Err(FetchError::Backoff);
            }
            let mut response = self
                .client
                .get(url.clone())
                .header(ACCEPT, kind.accept())
                .send()
                .await
                .map_err(|err| self.failed(&url, err))?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(FetchError::Status(status.as_u16()))?;
                url = url
                    .join(location)
                    .map_err(|_| FetchError::Refused("redirect"))?;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status(status.as_u16()));
            }
            let (content_type, charset) = media_type(
                response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or(""),
            );
            if !kind.allows(&content_type) {
                return Err(FetchError::ContentType);
            }
            let max = kind.max_bytes();
            if kind != Kind::Html
                && response
                    .content_length()
                    .is_some_and(|len| len > max as u64)
            {
                return Err(FetchError::TooLarge);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|err| self.failed(&url, err))?
            {
                body.extend_from_slice(&chunk);
                if body.len() > max {
                    if kind == Kind::Html {
                        body.truncate(max);
                        break;
                    }
                    return Err(FetchError::TooLarge);
                }
            }
            return Ok(Fetched {
                url,
                content_type,
                charset,
                body,
            });
        }
        Err(FetchError::TooManyRedirects)
    }

    /// The addresses `url` would be fetched from, or why it is refused:
    /// the checks of [`Self::get`] without the request.
    pub async fn vet(&self, url: &Url) -> Result<Vec<IpAddr>, FetchError> {
        self.guard.policy.check_url(url)?;
        match literal_ip(url) {
            Some(ip) => Ok(vec![ip]),
            None => {
                let domain = url.host_str().ok_or(FetchError::Refused("host"))?;
                self.guard.lookup(domain).await
            }
        }
    }

    /// Puts the host of `url` in backoff.
    fn slow_down(&self, url: &Url) {
        let key = match url.host() {
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            Some(Host::Domain(domain)) => normalized(domain),
            None => return,
        };
        let until = Instant::now() + BACKOFF;
        let mut slow = self.guard.slow.lock().expect("lock");
        if slow.len() >= BACKOFF_ENTRIES {
            let now = Instant::now();
            slow.retain(|_, until| *until > now);
            if slow.len() >= BACKOFF_ENTRIES {
                slow.clear();
            }
        }
        slow.insert(key, until);
    }

    /// The guard's own refusal when the resolver gave one; a timeout puts
    /// the host in backoff.
    fn failed(&self, url: &Url, err: reqwest::Error) -> FetchError {
        let mut source = std::error::Error::source(&err);
        while let Some(inner) = source {
            if let Some(refused) = inner.downcast_ref::<FetchError>() {
                return refused.clone();
            }
            source = inner.source();
        }
        if err.is_timeout() {
            self.slow_down(url);
            FetchError::Timeout
        } else {
            // Without the URL: the reason is logged at `info`, and the
            // query string of a message's link may hold a token.
            FetchError::Http(err.without_url().to_string())
        }
    }
}

impl Guard {
    /// `domain`'s addresses if it may be fetched: not a local or denied
    /// name, every address public, the name not in backoff.
    async fn lookup(&self, domain: &str) -> Result<Vec<IpAddr>, FetchError> {
        let domain = normalized(domain);
        if self.policy.denied_name(&domain) {
            return Err(FetchError::Refused("host"));
        }
        if self.in_backoff(&domain) {
            return Err(FetchError::Backoff);
        }
        let ips = match self.policy.hosts.get(&domain) {
            Some(fixed) => fixed.clone(),
            None => self.dns(&domain).await?,
        };
        if ips.is_empty() {
            return Err(FetchError::Dns);
        }
        // One private answer refuses the name: no picking the public one
        // of a mixed set, which a rebinding server could hand out.
        if !ips.iter().all(|ip| self.policy.allowed_ip(*ip)) {
            return Err(FetchError::Refused("address"));
        }
        Ok(ips)
    }

    async fn dns(&self, domain: &str) -> Result<Vec<IpAddr>, FetchError> {
        let permit = tokio::time::timeout(DNS_TIMEOUT, self.lookups.clone().acquire_owned())
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|_| FetchError::Dns)?;
        let name = domain.to_owned();
        // The permit goes with the lookup: a timeout here does not stop the
        // system resolver, so its slot frees only when that returns.
        let lookup = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            (name.as_str(), 0)
                .to_socket_addrs()
                .map(|addrs| addrs.map(|addr| addr.ip()).collect::<Vec<_>>())
        });
        let mut ips = tokio::time::timeout(DNS_TIMEOUT, lookup)
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|_| FetchError::Dns)?
            .map_err(|_| FetchError::Dns)?;
        ips.sort();
        ips.dedup();
        Ok(ips)
    }

    fn in_backoff(&self, host: &str) -> bool {
        self.slow
            .lock()
            .expect("lock")
            .get(host)
            .is_some_and(|until| *until > Instant::now())
    }
}

impl Policy {
    /// Everything about `url` that needs no DNS: scheme, credentials, port,
    /// local and denied names, literal addresses.
    pub fn check_url(&self, url: &Url) -> Result<(), FetchError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::Refused("scheme"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::Refused("credentials"));
        }
        let port = url
            .port_or_known_default()
            .ok_or(FetchError::Refused("port"))?;
        let standard = port == 80 || port == 443;
        match url.host().ok_or(FetchError::Refused("host"))? {
            Host::Domain(domain) => {
                if self.denied_name(&normalized(domain)) {
                    return Err(FetchError::Refused("host"));
                }
                if !standard && !self.trusted.iter().any(|addr| addr.port() == port) {
                    return Err(FetchError::Refused("port"));
                }
            }
            Host::Ipv4(ip) => self.check_literal(IpAddr::V4(ip), port, standard)?,
            Host::Ipv6(ip) => self.check_literal(IpAddr::V6(ip), port, standard)?,
        }
        Ok(())
    }

    fn check_literal(&self, ip: IpAddr, port: u16, standard: bool) -> Result<(), FetchError> {
        if self.trusted.contains(&SocketAddr::new(ip, port)) {
            return Ok(());
        }
        if !standard {
            return Err(FetchError::Refused("port"));
        }
        if !is_public(ip) {
            return Err(FetchError::Refused("address"));
        }
        Ok(())
    }

    fn allowed_ip(&self, ip: IpAddr) -> bool {
        is_public(ip) || self.trusted.iter().any(|addr| addr.ip() == ip)
    }

    fn denied_name(&self, domain: &str) -> bool {
        if !domain.contains('.') || LOCAL_SUFFIXES.iter().any(|s| domain.ends_with(s)) {
            return true;
        }
        self.deny_hosts.iter().any(|deny| {
            domain == deny
                || domain
                    .strip_suffix(deny.as_str())
                    .is_some_and(|rest| rest.ends_with('.'))
        })
    }
}

fn normalized(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

fn literal_ip(url: &Url) -> Option<IpAddr> {
    match url.host()? {
        Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        Host::Domain(_) => None,
    }
}

fn media_type(header: &str) -> (String, Option<String>) {
    let mut parts = header.split(';');
    let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let charset = parts.find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim().eq_ignore_ascii_case("charset"))
            .then(|| value.trim().trim_matches('"').to_ascii_lowercase())
    });
    (media, charset)
}

/// Whether `ip` is a globally routed unicast address: not loopback,
/// private, link-local, carrier-grade NAT, multicast, documentation,
/// benchmarking or otherwise reserved. IPv6 forms that embed an IPv4
/// address are judged by that address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..128).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (18..20).contains(&b))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    // NAT64 (64:ff9b::/96) reaches the embedded IPv4 address.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4] {
        let [a, b] = s[6].to_be_bytes();
        let [c, d] = s[7].to_be_bytes();
        return is_public_v4(Ipv4Addr::new(a, b, c, d));
    }
    // Only global unicast (2000::/3), minus the special blocks in it:
    // 2001::/23 (IETF: Teredo, ORCHID, benchmarking), 2001:db8::/32
    // (documentation) and 2002::/16 (6to4, which can embed anything).
    (s[0] & 0xe000) == 0x2000
        && !(s[0] == 0x2001 && s[1] < 0x0200)
        && !(s[0] == 0x2001 && s[1] == 0x0db8)
        && s[0] != 0x2002
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_and_special_addresses_are_not_public() {
        for s in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "2001::1",
            "2002:a00:1::1",
        ] {
            assert!(!is_public(ip(s)), "{s}");
        }
        for s in [
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
            "2606:4700:4700::1111",
            "2a00:1450:4001::200e",
        ] {
            assert!(is_public(ip(s)), "{s}");
        }
    }

    async fn vet(policy: &Policy, url: &str) -> Result<Vec<IpAddr>, FetchError> {
        Fetcher::new(policy.clone())
            .vet(&Url::parse(url).unwrap())
            .await
    }

    #[tokio::test]
    async fn refuses_internal_targets() {
        let mut policy = Policy {
            deny_hosts: vec!["minio.example.org".into()],
            ..Policy::default()
        };
        policy
            .hosts
            .insert("evil.example".into(), vec![ip("10.0.0.7")]);
        policy
            .hosts
            .insert("mixed.example".into(), vec![ip("1.1.1.1"), ip("127.0.0.1")]);
        policy
            .hosts
            .insert("fine.example".into(), vec![ip("93.184.216.34")]);
        for url in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[::ffff:7f00:1]/",
            "http://0x7f000001/",
            "http://2130706433/",
            "http://localhost/",
            "http://postgres:5432/",
            "http://minio/",
            "http://db.internal/",
            "http://printer.local/",
            "http://minio.example.org/",
            "http://bucket.minio.example.org/",
            "http://evil.example/",
            "http://mixed.example/",
            "http://fine.example:8080/",
            "http://user:pw@fine.example/",
            "ftp://fine.example/",
            "file:///etc/passwd",
        ] {
            assert!(vet(&policy, url).await.is_err(), "{url}");
        }
        assert_eq!(
            vet(&policy, "https://fine.example/x").await,
            Ok(vec![ip("93.184.216.34")])
        );
        assert_eq!(
            vet(&policy, "http://fine.example./").await,
            Ok(vec![ip("93.184.216.34")])
        );
    }

    /// The client connects only where its resolver, the guard, says: a
    /// name refused there fails the request with the guard's reason.
    #[tokio::test]
    async fn the_client_resolves_through_the_guard() {
        let mut policy = Policy::default();
        policy
            .hosts
            .insert("rebound.example".into(), vec![ip("127.0.0.1")]);
        let fetcher = Fetcher::new(policy);
        assert_eq!(
            fetcher
                .get(&Url::parse("http://rebound.example/").unwrap(), Kind::Html)
                .await
                .unwrap_err(),
            FetchError::Refused("address")
        );
    }

    #[tokio::test]
    async fn a_slow_host_is_skipped_for_a_while() {
        let mut policy = Policy::default();
        policy
            .hosts
            .insert("slow.example".into(), vec![ip("93.184.216.34")]);
        policy
            .hosts
            .insert("alias.example".into(), vec![ip("93.184.216.34")]);
        let fetcher = Fetcher::new(policy);
        let slow = Url::parse("https://slow.example/").unwrap();
        assert!(fetcher.vet(&slow).await.is_ok());
        fetcher.slow_down(&slow);
        assert_eq!(fetcher.vet(&slow).await, Err(FetchError::Backoff));
        // Another name on the same address (a CDN, shared hosting) is not
        // held up by it.
        assert!(
            fetcher
                .vet(&Url::parse("https://alias.example/").unwrap())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_connection_error_does_not_carry_the_url() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let fetcher = Fetcher::new(Policy {
            trusted: vec![closed],
            ..Policy::default()
        });
        let url = Url::parse(&format!("http://{closed}/page?token=secret")).unwrap();
        let err = fetcher.get(&url, Kind::Html).await.unwrap_err();
        assert_eq!(err.code(), "http", "{err}");
        assert!(!err.to_string().contains("secret"), "{err}");
    }

    /// TikTok and others give unknown agents a page without OpenGraph tags.
    #[test]
    fn the_agent_names_itself_like_known_preview_bots() {
        assert!(USER_AGENT.starts_with("Mozilla/5.0 (compatible; GelabberBot/"));
        assert!(USER_AGENT.ends_with("; like Twitterbot)"), "{USER_AGENT}");
    }

    #[test]
    fn media_type_and_charset() {
        assert_eq!(
            media_type("Text/HTML; Charset=\"ISO-8859-1\""),
            ("text/html".into(), Some("iso-8859-1".into()))
        );
        assert_eq!(media_type(""), (String::new(), None));
    }
}
