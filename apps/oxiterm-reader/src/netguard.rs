//! SSRF guard: which IP addresses and URLs the reader is allowed to talk to.
//!
//! Everything that is not a globally routable unicast address is refused:
//! loopback, private (RFC 1918), link-local, CGNAT, multicast, unspecified,
//! reserved/documentation/benchmark ranges, and IPv6 equivalents including
//! IPv4-mapped / compatible / NAT64 / 6to4 forms (the embedded IPv4 is vetted).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

/// True when `ip` is a public, globally routable unicast address.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

/// True when `ip` is a public IPv4 address.
pub fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    if ip.is_unspecified() || ip.is_loopback() || ip.is_private() || ip.is_link_local() {
        return false;
    }
    if ip.is_multicast() || ip.is_broadcast() || ip.is_documentation() {
        return false;
    }
    // 0.0.0.0/8 "this network"
    if o[0] == 0 {
        return false;
    }
    // 100.64.0.0/10 CGNAT
    if o[0] == 100 && (o[1] & 0xC0) == 64 {
        return false;
    }
    // 192.0.0.0/24 IETF protocol assignments
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return false;
    }
    // 198.18.0.0/15 benchmarking
    if o[0] == 198 && (o[1] & 0xFE) == 18 {
        return false;
    }
    // 240.0.0.0/4 reserved (includes broadcast)
    if o[0] >= 240 {
        return false;
    }
    true
}

/// True when `ip` is a public IPv6 address.
pub fn is_public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }
    let s = ip.segments();
    // IPv4-mapped ::ffff:a.b.c.d
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // ::/96 (IPv4-compatible, deprecated) and ::/8 in general: refuse.
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return false;
    }
    // 64:ff9b::/96 NAT64: vet the embedded IPv4.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
        return is_public_v4(embedded_v4(s[6], s[7]));
    }
    // 64:ff9b:1::/48 local-use NAT64
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0x0001 {
        return false;
    }
    // 100::/64 discard-only
    if s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0 {
        return false;
    }
    // fc00::/7 unique local
    if (s[0] & 0xFE00) == 0xFC00 {
        return false;
    }
    // fe80::/10 link-local, fec0::/10 site-local (deprecated)
    if (s[0] & 0xFFC0) == 0xFE80 || (s[0] & 0xFFC0) == 0xFEC0 {
        return false;
    }
    // 2001:db8::/32 documentation, 2001::/32 Teredo, 2001:10::/28 ORCHID
    if s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0x0000 || (s[1] & 0xFFF0) == 0x0010) {
        return false;
    }
    // 2002::/16 6to4: vet the embedded IPv4.
    if s[0] == 0x2002 {
        return is_public_v4(embedded_v4(s[1], s[2]));
    }
    // Only 2000::/3 is global unicast today.
    (s[0] & 0xE000) == 0x2000
}

fn embedded_v4(hi: u16, lo: u16) -> Ipv4Addr {
    Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
}

/// Why a URL was refused before any network traffic happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRefusal {
    /// Not parseable as an absolute URL.
    Malformed,
    /// Scheme other than http/https.
    Scheme,
    /// URL carries `user:pass@`.
    UserInfo,
    /// Port other than 80/443.
    Port,
    /// No host component.
    NoHost,
    /// Host is a non-public IP literal or a local-only name.
    PrivateHost,
    /// Host is not on the allow-list (public instances).
    NotAllowed,
    /// URL longer than the cap.
    TooLong,
}

/// Maximum accepted URL length.
pub const MAX_URL_LEN: usize = 2048;

/// Static (pre-DNS) validation of a URL. DNS results are vetted separately by the
/// connection-time resolver (see `fetch::VettedResolver`).
pub fn validate_url(url: &Url, allowed_hosts: Option<&[String]>) -> Result<(), UrlRefusal> {
    if url.as_str().len() > MAX_URL_LEN {
        return Err(UrlRefusal::TooLong);
    }
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(UrlRefusal::Scheme),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlRefusal::UserInfo);
    }
    if let Some(p) = url.port() {
        if p != 80 && p != 443 {
            return Err(UrlRefusal::Port);
        }
    }
    match url.host() {
        None => return Err(UrlRefusal::NoHost),
        Some(Host::Ipv4(ip)) => {
            if !is_public_v4(ip) {
                return Err(UrlRefusal::PrivateHost);
            }
        }
        Some(Host::Ipv6(ip)) => {
            if !is_public_v6(ip) {
                return Err(UrlRefusal::PrivateHost);
            }
        }
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.ends_with(".lan")
                || !d.contains('.')
            {
                return Err(UrlRefusal::PrivateHost);
            }
        }
    }
    if let Some(allowed) = allowed_hosts {
        let host = url.host_str().unwrap_or("").trim_end_matches('.').to_ascii_lowercase();
        if !host_allowed(&host, allowed) {
            return Err(UrlRefusal::NotAllowed);
        }
    }
    Ok(())
}

/// Suffix match on dot boundaries: `en.wikipedia.org` matches `wikipedia.org`.
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|a| {
        let a = a.trim().trim_start_matches('.').to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_v4_private_ranges() {
        for s in [
            "0.0.0.0", "0.1.2.3", "10.0.0.1", "127.0.0.1", "127.255.255.254", "169.254.169.254",
            "172.16.0.1", "172.31.255.255", "192.168.1.1", "100.64.0.1", "100.127.255.255",
            "198.18.0.1", "198.19.255.255", "192.0.0.8", "192.0.2.1", "198.51.100.7",
            "203.0.113.9", "224.0.0.1", "239.255.255.255", "240.0.0.1", "255.255.255.255",
        ] {
            assert!(!is_public_ip(ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn allows_v4_public() {
        for s in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "100.63.255.255", "100.128.0.1", "172.32.0.1", "198.20.0.1"] {
            assert!(is_public_ip(ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn blocks_v6_private_and_embedded() {
        for s in [
            "::", "::1", "::127.0.0.1", "::ffff:127.0.0.1", "::ffff:10.1.2.3", "::ffff:169.254.169.254",
            "64:ff9b::7f00:1", "64:ff9b::a00:1", "64:ff9b:1::1", "fc00::1", "fd12:3456::1", "fe80::1",
            "febf::1", "fec0::1", "ff02::1", "2001:db8::1", "2001::1", "2002:7f00:1::1", "2002:c0a8:101::1",
            "100::1", "4000::1", "::2",
        ] {
            assert!(!is_public_ip(ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn allows_v6_public() {
        for s in ["2606:4700:4700::1111", "2a00:1450:4001:81b::200e", "64:ff9b::808:808", "::ffff:8.8.8.8", "2002:0808:0808::1"] {
            assert!(is_public_ip(ip(s)), "{s} must be allowed");
        }
    }

    fn check(u: &str) -> Result<(), UrlRefusal> {
        validate_url(&Url::parse(u).unwrap(), None)
    }

    #[test]
    fn url_scheme_userinfo_port() {
        assert_eq!(check("ftp://example.com/"), Err(UrlRefusal::Scheme));
        assert_eq!(check("file:///etc/passwd"), Err(UrlRefusal::Scheme));
        assert_eq!(check("gopher://example.com/"), Err(UrlRefusal::Scheme));
        assert_eq!(check("https://user:pw@example.com/"), Err(UrlRefusal::UserInfo));
        assert_eq!(check("https://user@example.com/"), Err(UrlRefusal::UserInfo));
        assert_eq!(check("http://example.com:22/"), Err(UrlRefusal::Port));
        assert_eq!(check("http://example.com:8080/"), Err(UrlRefusal::Port));
        assert_eq!(check("https://example.com:443/x"), Ok(()));
        assert_eq!(check("http://example.com/x?y=1"), Ok(()));
    }

    #[test]
    fn url_ip_literals_in_every_spelling() {
        // The `url` crate normalises decimal/hex/octal IPv4 spellings to dotted form.
        assert_eq!(check("http://127.0.0.1/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://2130706433/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://0x7f000001/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://0177.0.0.1/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://127.1/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://[::1]/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://[::ffff:7f00:1]/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://[fe80::1]/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://169.254.169.254/latest/meta-data/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://10.0.0.5/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://100.64.1.1/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://localhost/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://LOCALHOST./"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://foo.localhost/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://printer.local/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://intranet/"), Err(UrlRefusal::PrivateHost));
        assert_eq!(check("http://8.8.8.8/"), Ok(()));
    }

    #[test]
    fn url_allow_list_is_suffix_on_dot_boundary() {
        let allowed = vec!["wikipedia.org".to_string()];
        let ok = |u: &str| validate_url(&Url::parse(u).unwrap(), Some(&allowed));
        assert_eq!(ok("https://en.wikipedia.org/wiki/X"), Ok(()));
        assert_eq!(ok("https://wikipedia.org/"), Ok(()));
        assert_eq!(ok("https://evilwikipedia.org/"), Err(UrlRefusal::NotAllowed));
        assert_eq!(ok("https://wikipedia.org.evil.com/"), Err(UrlRefusal::NotAllowed));
    }

    #[test]
    fn url_too_long() {
        let u = format!("https://example.com/{}", "a".repeat(3000));
        assert_eq!(check(&u), Err(UrlRefusal::TooLong));
    }
}
