//! User identity determination and reserved-key injection.

use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    SshKey,
    SshPassword,
    TrustedHeader,
    Guest,
}

#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub username: String,
    pub auth_method: AuthMethod,
}

impl UserIdentity {
    pub fn ssh_key(username: impl Into<String>) -> Self {
        Self { username: username.into(), auth_method: AuthMethod::SshKey }
    }
    pub fn ssh_password(username: impl Into<String>) -> Self {
        Self { username: username.into(), auth_method: AuthMethod::SshPassword }
    }
    pub fn ssh_no_auth(username: impl Into<String>) -> Self {
        Self { username: username.into(), auth_method: AuthMethod::Guest }
    }
    pub fn guest() -> Self {
        Self { username: "guest".to_string(), auth_method: AuthMethod::Guest }
    }

    pub fn from_trusted_header(
        header_value: &str,
        peer_addr: std::net::SocketAddr,
        trusted_proxy: &str,
    ) -> Option<Self> {
        let peer_ip = peer_addr.ip().to_string();
        let proxy_ip = trusted_proxy.split(':').next().unwrap_or(trusted_proxy);
        if peer_ip == proxy_ip {
            Some(Self {
                username: header_value.to_string(),
                auth_method: AuthMethod::TrustedHeader,
            })
        } else {
            warn!(
                "X-Forwarded-User header from untrusted peer {} (configured proxy: {}); ignoring",
                peer_addr, trusted_proxy
            );
            None
        }
    }

    pub fn inject_reserved_keys(&self, state: &mut crate::state::StateManager) {
        state.set(
            "_username".to_string(),
            crate::state::StateValue::Str(self.username.clone()),
        );
        state.set(
            "_auth_method".to_string(),
            crate::state::StateValue::Str(format!("{:?}", self.auth_method)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, IpAddr, Ipv4Addr};

    fn make_addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port)
    }

    #[test]
    fn test_10_trusted_header_accepted() {
        let proxy = "192.168.1.1:8080";
        let peer = make_addr([192, 168, 1, 1], 54321);
        let id = UserIdentity::from_trusted_header("alice", peer, proxy).unwrap();
        assert_eq!(id.username, "alice");
        assert_eq!(id.auth_method, AuthMethod::TrustedHeader);
    }

    #[test]
    fn test_11_trusted_header_rejected_wrong_peer() {
        let proxy = "192.168.1.1";
        let peer = make_addr([10, 0, 0, 1], 54321);
        let id = UserIdentity::from_trusted_header("attacker", peer, proxy);
        assert!(id.is_none());
    }

    #[test]
    fn test_12_no_header_guest_and_none() {
        let g = UserIdentity::guest();
        assert_eq!(g.username, "guest");
        assert_eq!(g.auth_method, AuthMethod::Guest);

        let mut sm = crate::state::StateManager::new();
        g.inject_reserved_keys(&mut sm);
        assert_eq!(sm.get("_username"), Some(&crate::state::StateValue::Str("guest".to_string())));
    }
}
