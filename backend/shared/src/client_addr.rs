//! The client address both binaries key their per-client limits by: the API
//! its rate limits, the SFU its cap on sockets that have not joined yet.
//!
//! Caddy, in front of the API and the SFU in every supported setup, sends
//! `X-Forwarded-For` as the one client address it resolved: its peer, or,
//! when that peer is a proxy it trusts, the rightmost entry that is no
//! trusted proxy. A proxy that appends instead puts its own peer last. So the
//! rightmost entry is the one written by the proxy next to the service;
//! entries to its left come from the client and are not believed, and a
//! rightmost entry that is no address falls back to the TCP peer, never to
//! one further left. Without the header (the service reached directly) the
//! TCP peer is the client.

use std::net::{IpAddr, Ipv6Addr};

/// The key for one client, IPv6 grouped per /64 (one subscriber's prefix,
/// like one IPv4 address behind NAT). `forwarded_for` holds every
/// `X-Forwarded-For` header value in order; `peer` is the TCP peer, if known.
pub fn client_key<'a>(
    forwarded_for: impl IntoIterator<Item = &'a str>,
    peer: Option<IpAddr>,
) -> String {
    forwarded_ip(forwarded_for)
        .or(peer)
        .map(rate_key)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn forwarded_ip<'a>(forwarded_for: impl IntoIterator<Item = &'a str>) -> Option<IpAddr> {
    forwarded_for
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .last()?
        .parse()
        .ok()
}

/// IPv4 as is (also when it arrives IPv4-mapped); IPv6 as its /64, since a
/// single client usually holds a whole prefix and picks addresses from it.
fn rate_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{}/64", Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_for_takes_the_rightmost_ip() {
        assert_eq!(client_key(["10.0.0.9, 10.0.0.1"], None), "10.0.0.1");
        assert_eq!(
            client_key(["10.0.0.9, 10.0.0.1", "10.0.0.2"], None),
            "10.0.0.2"
        );
        assert_eq!(client_key(["10.0.0.1, "], None), "10.0.0.1");
    }

    #[test]
    fn without_forwarded_for_the_peer_is_the_client() {
        let peer = Some(IpAddr::from([192, 0, 2, 7]));
        assert_eq!(client_key([], peer), "192.0.2.7");
        assert_eq!(client_key([], None), "unknown");
    }

    #[test]
    fn ipv6_clients_are_grouped_per_64() {
        let one = client_key(["2001:db8:1:2::1"], None);
        assert_eq!(one, "2001:db8:1:2::/64");
        assert_eq!(client_key(["2001:db8:1:2:ffff::9"], None), one);
        assert_eq!(client_key(["2001:db8:1:3::1"], None), "2001:db8:1:3::/64");
        let peer = Some("2001:db8:1:2:abcd::5".parse().unwrap());
        assert_eq!(client_key([], peer), one);
    }

    #[test]
    fn ipv4_mapped_addresses_count_as_ipv4() {
        assert_eq!(client_key(["::ffff:1.2.3.4"], None), "1.2.3.4");
        let peer = Some("::ffff:1.2.3.4".parse().unwrap());
        assert_eq!(client_key([], peer), "1.2.3.4");
    }

    #[test]
    fn an_unparsable_rightmost_entry_falls_back_to_the_peer() {
        let peer = Some(IpAddr::from([192, 0, 2, 7]));
        // Never the entry further left: the client wrote that one.
        assert_eq!(client_key(["10.0.0.9, not-an-ip"], peer), "192.0.2.7");
        assert_eq!(client_key(["10.0.0.9, not-an-ip"], None), "unknown");
    }
}
