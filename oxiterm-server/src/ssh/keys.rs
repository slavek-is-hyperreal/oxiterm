use std::path::Path;
use russh_keys::key;
use anyhow::{Context, Result};
use std::fs;
use tracing::{info, warn};

pub struct AuthorizedKeys {
    pub keys: Vec<key::PublicKey>,
}

impl AuthorizedKeys {
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
            if let Ok(key) = russh_keys::parse_public_key_base64(line) {
                keys.push(key);
            }
        }
        Ok(Self { keys })
    }

    pub fn verify(&self, key: &key::PublicKey) -> bool {
        self.keys.contains(key)
    }
}

pub fn load_host_key(path: &Path) -> Result<key::KeyPair> {
    if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read host key: {}", path.display()))?;
        // Try to parse existing key
        if let Ok(key) = russh_keys::decode_secret_key(&content, None) {
            return Ok(key);
        }
        warn!("Failed to parse existing host key at {}, generating new one", path.display());
    }

    info!("Generating new host key at {}", path.display());
    let key = key::KeyPair::generate_ed25519()
        .ok_or_else(|| anyhow::anyhow!("Failed to generate host key"))?;
    
    // Save key to disk
    let mut secret_key = Vec::new();
    russh_keys::encode_pkcs8_pem(&key, &mut secret_key)
        .context("Failed to encode host key")?;
    fs::write(path, secret_key)
        .with_context(|| format!("Failed to write host key to {}", path.display()))?;
    
    // Set permissions (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }

    Ok(key)
}
