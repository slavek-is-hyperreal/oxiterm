//! URL validation helper to prevent SSRF.
//!
//! Validates schemes and blocks metadata/link-local IPv4/IPv6 addresses and domain patterns.

use url::Url;

/// Validates that a URL is HTTP/HTTPS and does not target link-local/cloud metadata IP space.
///
/// Reduces Server-Side Request Forgery (SSRF) risks on backend redirects.
pub fn validate_app_server_url(url_str: &str) -> anyhow::Result<()> {
    let parsed = Url::parse(url_str)?;
    
    // 1. Check scheme
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        anyhow::bail!("Unsupported URL scheme: {}. Only http and https are allowed.", scheme);
    }
    
    // 2. Check host
    if let Some(host) = parsed.host() {
        match host {
            url::Host::Ipv4(ip) => {
                if is_link_local_ipv4(ip) {
                    anyhow::bail!("URL host resolves to a forbidden link-local/metadata IP address: {}", ip);
                }
            }
            url::Host::Ipv6(ip) => {
                if is_link_local_ipv6(ip) {
                    anyhow::bail!("URL host resolves to a forbidden link-local/metadata IP address: {}", ip);
                }
            }
            url::Host::Domain(domain) => {
                let domain_lower = domain.to_lowercase();
                if domain_lower.contains("metadata") || domain_lower.contains("169.254") {
                    anyhow::bail!("URL domain is forbidden: {}", domain);
                }
            }
        }
    } else {
        anyhow::bail!("URL has no host");
    }
    
    Ok(())
}

fn is_link_local_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 169 && octets[1] == 254
}

fn is_link_local_ipv6(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}
