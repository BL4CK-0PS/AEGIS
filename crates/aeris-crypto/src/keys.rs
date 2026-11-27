//! Ed25519 identities, domain-separated signing, and the public-key registry.
//!
//! # Domain separation
//!
//! The same server key signs commands *and* audit hashes, so every signature
//! covers `tag || message` with a distinct tag per message type. A signature
//! made for one purpose can never verify as another:
//!
//! | Domain | Tag | Signed by |
//! |---|---|---|
//! | [`Domain::Event`]   | `AERIS-v1/event\0`   | node key |
//! | [`Domain::Command`] | `AERIS-v1/command\0` | server key |
//! | [`Domain::Ack`]     | `AERIS-v1/ack\0`     | simulator controller key |
//! | [`Domain::Audit`]   | `AERIS-v1/audit\0`   | server key |
//!
//! Verification uses `verify_strict` (rejects small-order keys and
//! non-canonical signatures) and the registry refuses weak public keys.

use crate::CryptoError;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use zeroize::Zeroizing;

/// Message domain for signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// Event bodies (`event.proto` `Body`).
    Event,
    /// Command bodies (`command.proto` `CommandBody`).
    Command,
    /// Command acknowledgements (`command.proto` `CommandAck`).
    Ack,
    /// Audit chain hashes `h_n`.
    Audit,
}

impl Domain {
    /// The domain-separation tag.
    pub fn tag(self) -> &'static [u8] {
        match self {
            Domain::Event => b"AERIS-v1/event\0",
            Domain::Command => b"AERIS-v1/command\0",
            Domain::Ack => b"AERIS-v1/ack\0",
            Domain::Audit => b"AERIS-v1/audit\0",
        }
    }

    fn message(self, bytes: &[u8]) -> Vec<u8> {
        let tag = self.tag();
        let mut m = Vec::with_capacity(tag.len() + bytes.len());
        m.extend_from_slice(tag);
        m.extend_from_slice(bytes);
        m
    }
}

/// Identity names that are not nodes.
pub mod ids {
    /// Control-plane key: signs commands and audit hashes.
    pub const SERVER: &str = "server";
    /// Simulator controller key: signs command acknowledgements.
    pub const SIM_CONTROLLER: &str = "sim-controller";
}

/// A named Ed25519 signing identity. The secret is zeroized on drop
/// (`ed25519-dalek` `zeroize` feature) and never printed.
pub struct SigningIdentity {
    id: String,
    key: SigningKey,
}

impl fmt::Debug for SigningIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningIdentity")
            .field("id", &self.id)
            .field("public", &hex::encode(self.key.verifying_key().as_bytes()))
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl SigningIdentity {
    /// Generates a fresh identity from the operating system CSPRNG.
    pub fn generate(id: impl Into<String>) -> Result<Self, CryptoError> {
        let id = id.into();
        aeris_wire::validate_entity_id("key id", &id)?;
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(seed.as_mut()).map_err(|e| CryptoError::Rng(e.to_string()))?;
        Ok(Self {
            id,
            key: SigningKey::from_bytes(&seed),
        })
    }

    /// Builds an identity from a 32-byte secret seed.
    pub fn from_seed(id: impl Into<String>, seed: &[u8; 32]) -> Result<Self, CryptoError> {
        let id = id.into();
        aeris_wire::validate_entity_id("key id", &id)?;
        Ok(Self {
            id,
            key: SigningKey::from_bytes(seed),
        })
    }

    /// Parses a secret from 64 hex characters (surrounding whitespace allowed).
    pub fn from_secret_hex(id: impl Into<String>, s: &str) -> Result<Self, CryptoError> {
        let mut seed = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(s.trim(), seed.as_mut())
            .map_err(|_| CryptoError::KeyFormat("secret must be 64 hex characters".into()))?;
        Self::from_seed(id, &seed)
    }

    /// Loads a secret key file written by [`SigningIdentity::write_secret_file`].
    ///
    /// On Unix the file must not be readable or writable by group or others
    /// (mode `0600` or stricter); a looser mode is an error, not a warning.
    pub fn load_secret_file(id: impl Into<String>, path: &Path) -> Result<Self, CryptoError> {
        let id = id.into();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(path)
                .map_err(|e| CryptoError::Io(format!("{}: {e}", path.display())))?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(CryptoError::KeyFormat(format!(
                    "{}: permissions {:o} are too open; secret keys must be 0600",
                    path.display(),
                    mode & 0o777
                )));
            }
        }
        let text = Zeroizing::new(
            std::fs::read_to_string(path)
                .map_err(|e| CryptoError::Io(format!("{}: {e}", path.display())))?,
        );
        Self::from_secret_hex(id, &text)
    }

    /// Writes the secret as hex to `path`, created with mode `0600`. Refuses
    /// to overwrite an existing file.
    pub fn write_secret_file(&self, path: &Path) -> Result<(), CryptoError> {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(path)
            .map_err(|e| CryptoError::Io(format!("{}: {e}", path.display())))?;
        let hexed = Zeroizing::new(hex::encode(self.key.to_bytes()));
        f.write_all(hexed.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| CryptoError::Io(format!("{}: {e}", path.display())))
    }

    /// Identity name.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Public key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    /// Public key as lowercase hex.
    pub fn public_hex(&self) -> String {
        hex::encode(self.key.verifying_key().as_bytes())
    }

    /// Signs `tag(domain) || bytes`.
    pub fn sign(&self, domain: Domain, bytes: &[u8]) -> [u8; 64] {
        self.key.sign(&domain.message(bytes)).to_bytes()
    }
}

/// Verifies `signature` over `tag(domain) || bytes` with strict Ed25519
/// verification.
pub fn verify(
    key: &VerifyingKey,
    domain: Domain,
    bytes: &[u8],
    signature: &[u8; 64],
) -> Result<(), CryptoError> {
    let sig = Signature::from_bytes(signature);
    key.verify_strict(&domain.message(bytes), &sig)
        .map_err(|_| CryptoError::BadSignature)
}

/// Parses a public key from 64 hex characters, rejecting weak (small-order)
/// keys.
pub fn parse_public_hex(s: &str) -> Result<VerifyingKey, CryptoError> {
    let mut b = [0u8; 32];
    hex::decode_to_slice(s.trim(), &mut b)
        .map_err(|_| CryptoError::KeyFormat("public key must be 64 hex characters".into()))?;
    let k = VerifyingKey::from_bytes(&b)
        .map_err(|_| CryptoError::KeyFormat("not a valid Ed25519 point".into()))?;
    if k.is_weak() {
        return Err(CryptoError::KeyFormat(
            "weak (small-order) public key".into(),
        ));
    }
    Ok(k)
}

/// The trusted public keys: nodes, server, simulator controller.
///
/// Distributed as a TOML file (`[keys] node-a = "<hex>"`), produced by
/// `aeris-simd keygen` and mounted read-only into every service. Revocation
/// (isolating a node "stops accepting B's key") is runtime state.
#[derive(Debug, Clone, Default)]
pub struct KeyRegistry {
    keys: BTreeMap<String, VerifyingKey>,
    revoked: std::collections::BTreeSet<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    keys: BTreeMap<String, String>,
}

impl KeyRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses the TOML registry format.
    pub fn from_toml(text: &str) -> Result<Self, CryptoError> {
        let f: RegistryFile =
            toml::from_str(text).map_err(|e| CryptoError::KeyFormat(e.to_string()))?;
        let mut r = Self::new();
        for (id, hexkey) in f.keys {
            r.insert(&id, parse_public_hex(&hexkey)?)?;
        }
        Ok(r)
    }

    /// Serializes to the TOML registry format.
    pub fn to_toml(&self) -> String {
        let f = RegistryFile {
            keys: self
                .keys
                .iter()
                .map(|(id, k)| (id.clone(), hex::encode(k.as_bytes())))
                .collect(),
        };
        // BTreeMap<String,String> always serializes.
        toml::to_string(&f).unwrap_or_default()
    }

    /// Adds a key. Ids must be valid entity ids; duplicates are an error.
    pub fn insert(&mut self, id: &str, key: VerifyingKey) -> Result<(), CryptoError> {
        aeris_wire::validate_entity_id("key id", id)?;
        if key.is_weak() {
            return Err(CryptoError::KeyFormat(format!("{id}: weak public key")));
        }
        if self.keys.insert(id.to_string(), key).is_some() {
            return Err(CryptoError::KeyFormat(format!("{id}: duplicate key id")));
        }
        Ok(())
    }

    /// The key for `id`, unless unknown or revoked.
    pub fn get(&self, id: &str) -> Result<&VerifyingKey, CryptoError> {
        if self.revoked.contains(id) {
            return Err(CryptoError::Revoked(id.to_string()));
        }
        self.keys
            .get(id)
            .ok_or_else(|| CryptoError::UnknownKey(id.to_string()))
    }

    /// `true` if `id` has a registered key (revoked or not).
    pub fn contains(&self, id: &str) -> bool {
        self.keys.contains_key(id)
    }

    /// Stops accepting `id`'s signatures.
    pub fn revoke(&mut self, id: &str) {
        self.revoked.insert(id.to_string());
    }

    /// Accepts `id`'s signatures again.
    pub fn reinstate(&mut self, id: &str) {
        self.revoked.remove(id);
    }

    /// Registered ids.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }
}
