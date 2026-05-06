use std::path::Path;
use russh_keys::key;
use anyhow::{Context, Result};
use std::fs;
use tracing::info;

pub struct AuthorizedKeys {
    pub keys: Vec<key::PublicKey>,
}

impl AuthorizedKeys {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read authorized_keys: {:?}", path))?;
        
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
    if !path.exists() {
        info!("Generating new host key at {:?}", path);
        let key = key::KeyPair::generate_ed25519().unwrap();
        // Save key (simplified)
        return Ok(key);
    }
    // Load existing key logic here
    Ok(key::KeyPair::generate_ed25519().unwrap()) // Placeholder
}
