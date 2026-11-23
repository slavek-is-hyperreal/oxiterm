//! SSH keys loader and generator.
//!
//! Provides utilities to load and parse SSH authorized public keys for client authentication,
//! and to read or dynamically generate ED25519 host key pairs with restricted filesystem permissions.

use std::path::Path;
use russh::keys::{PrivateKey, PublicKey};
use anyhow::{Context, Result};
use std::fs;
use tracing::{info, warn};

/// Collection of public keys authorized for client passwordless SSH connections.
pub struct AuthorizedKeys {
    /// List of parsed public keys.
    pub keys: Vec<PublicKey>,
}

impl AuthorizedKeys {
    /// Loads public keys from an `authorized_keys` formatted file.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self { keys: Vec::new() });
        }
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read authorized_keys: {}", path.display()))?;
        
        let mut keys = Vec::new();
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            let base64_str = if parts.len() >= 2 {
                parts[1]
            } else {
                line
            };
            if let Ok(key) = russh::keys::parse_public_key_base64(base64_str) {
                keys.push(key);
            }
        }
        Ok(Self { keys })
    }

    /// Verifies if the presented client public key is authorized.
    ///
    /// Compares the key material only (the comment field is not significant).
    pub fn verify(&self, key: &PublicKey) -> bool {
        self.keys.iter().any(|k| k.key_data() == key.key_data())
    }
}

/// Loads the SSH server host key pair, generating a new ED25519 key if none exists.
///
/// Restricts file permissions to read/write for the owner only (`0o600` on Unix systems).
pub fn load_host_key(path: &Path) -> Result<PrivateKey> {
    if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read host key: {}", path.display()))?;
        if let Ok(key) = russh::keys::decode_secret_key(&content, None) {
            return Ok(key);
        }
        warn!("Failed to parse existing host key at {}, generating new one", path.display());
    }

    info!("Generating new host key at {}", path.display());
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), russh::keys::Algorithm::Ed25519)
        .map_err(|e| anyhow::anyhow!("Failed to generate host key: {e}"))?;
    
    let mut secret_key = Vec::new();
    russh::keys::encode_pkcs8_pem(&key, &mut secret_key)
        .context("Failed to encode host key")?;
    fs::write(path, secret_key)
        .with_context(|| format!("Failed to write host key to {}", path.display()))?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }

    Ok(key)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_key() {
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ/vG24eD+1GRR4GAfJ25RUpeo5GcPV8RmPY7r/RznqC slavekm@slavekm-desktop";
        let parts: Vec<&str> = line.split_whitespace().collect();
        let base64_str = if parts.len() >= 2 {
            parts[1]
        } else {
            line
        };
        let res = russh::keys::parse_public_key_base64(base64_str);
        assert!(res.is_ok(), "Failed to parse standard key: {:?}", res);
    }

    #[test]
    fn test_load_existing_host_key_format() {
        // The repo's PKCS#8 ED25519 host key must keep loading across russh upgrades
        // (a parse failure would silently regenerate the host identity).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../host_key");
        if !path.exists() {
            return;
        }
        let content = std::fs::read_to_string(&path).unwrap();
        let key = russh::keys::decode_secret_key(&content, None).expect("host_key must parse");
        assert_eq!(key.algorithm(), russh::keys::Algorithm::Ed25519);
    }

    #[test]
    fn test_generated_host_key_roundtrip() {
        let dir = std::env::temp_dir().join(format!("oxiterm-hk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("host_key");
        let _ = std::fs::remove_file(&path);
        let generated = super::load_host_key(&path).unwrap();
        let reloaded = super::load_host_key(&path).unwrap();
        assert_eq!(generated.public_key().key_data(), reloaded.public_key().key_data());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
