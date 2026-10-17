//! Outbound requests for link previews, guarded against SSRF. A message can
//! name any URL, so before every request, redirects included, the host is
//! resolved here and every address must be public; the connection is then
//! pinned to the vetted address so a second DNS answer cannot move it.
//! Only ports 80 and 443, no credentials, no cookies, no proxy, small
//! bodies and short timeouts.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use url::{Host, Url};

/// Each request, connect to last body byte.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
pub const MAX_REDIRECTS: usize = 3;
/// Pages: only the head matters, so a longer page is cut, not refused.
pub const HTML_MAX_BYTES: usize = 1024 * 1024;
pub const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const JSON_MAX_BYTES: usize = 256 * 1024;
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; GelabberBot/",
    env!("CARGO_PKG_VERSION"),
    "; +https://github.com/Till-und-Scholler-AI-slop/Gelabber)"
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
    /// Tests only: socket addresses treated as public, any port. Empty in
    /// production, where there is no way to set it.
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
}

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

impl Policy {
    /// GET `url` as `kind`, following up to [`MAX_REDIRECTS`] redirects,
    /// each vetted like the first request.
    pub async fn get(&self, url: &Url, kind: Kind) -> Result<Fetched, FetchError> {
        let mut url = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let addr = self.vet(&url).await?;
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .user_agent(USER_AGENT);
            if let Some(Host::Domain(domain)) = url.host() {
                builder = builder.resolve(domain, addr);
            }
            let client = builder
                .build()
                .map_err(|err| FetchError::Http(err.to_string()))?;
            let mut response = client
                .get(url.clone())
                .header(ACCEPT, kind.accept())
                .send()
                .await
                .map_err(request_error)?;
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
            while let Some(chunk) = response.chunk().await.map_err(request_error)? {
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

    /// The address to connect to for `url`, or why it is refused.
    pub async fn vet(&self, url: &Url) -> Result<SocketAddr, FetchError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::Refused("scheme"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::Refused("credentials"));
        }
        let port = url
            .port_or_known_default()
            .ok_or(FetchError::Refused("port"))?;
        let candidates: Vec<IpAddr> = match url.host().ok_or(FetchError::Refused("host"))? {
            Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
            Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
            Host::Domain(domain) => {
                let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                if self.denied_name(&domain) {
                    return Err(FetchError::Refused("host"));
                }
                self.resolve(&domain, port).await?
            }
        };
        if candidates.is_empty() {
            return Err(FetchError::Dns);
        }
        let trusted = |ip: &IpAddr| self.trusted.contains(&SocketAddr::new(*ip, port));
        if !candidates.iter().all(trusted) {
            if port != 80 && port != 443 {
                return Err(FetchError::Refused("port"));
            }
            // One private answer refuses the name: no picking the public one
            // of a mixed set, which a rebinding server could hand out.
            if !candidates.iter().all(|ip| trusted(ip) || is_public(*ip)) {
                return Err(FetchError::Refused("address"));
            }
        }
        Ok(SocketAddr::new(candidates[0], port))
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

    async fn resolve(&self, domain: &str, port: u16) -> Result<Vec<IpAddr>, FetchError> {
        if let Some(fixed) = self.hosts.get(domain) {
            return Ok(fixed.clone());
        }
        let found = tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((domain, port)))
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|_| FetchError::Dns)?;
        let mut ips: Vec<IpAddr> = found.map(|addr| addr.ip()).collect();
        ips.dedup();
        Ok(ips)
    }
}

fn request_error(err: reqwest::Error) -> FetchError {
    if err.is_timeout() {
        FetchError::Timeout
    } else {
        FetchError::Http(err.to_string())
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

    async fn vet(policy: &Policy, url: &str) -> Result<SocketAddr, FetchError> {
        policy.vet(&Url::parse(url).unwrap()).await
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
            Ok("93.184.216.34:443".parse().unwrap())
        );
        assert_eq!(
            vet(&policy, "http://fine.example./").await,
            Ok("93.184.216.34:80".parse().unwrap())
        );
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
