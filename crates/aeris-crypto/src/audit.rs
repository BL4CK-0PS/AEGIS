//! Tamper-evident decision audit chain (§6.2, §6.15).
//!
//! ```text
//! h_0 = 0^256,   h_n = BLAKE3(h_{n-1} || bytes(D_n)),   σ_n = Sign_srv("AERIS-v1/audit\0" || h_n)
//! ```
//!
//! `bytes(D_n)` is produced by [`canonical_json`]: JSON with object keys in
//! sorted order, no insignificant whitespace, shortest round-trip floats, and
//! NaN/∞ refused. The same record always yields the same bytes.
//!
//! [`verify_chain`] detects, and reports the first index of: modified records,
//! modified hashes, invalid signatures, missing records (gaps in `n`),
//! reordered records, and — when given a signed [`Checkpoint`] — truncation of
//! the tail, which a hash chain alone cannot detect.

use crate::CryptoError;
use crate::keys::{Domain, SigningIdentity, verify};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

/// `h_0`.
pub const GENESIS: [u8; 32] = [0u8; 32];

/// Deterministic serialization of any record (`bytes(D_n)`).
pub fn canonical_json<T: Serialize>(record: &T) -> Result<Vec<u8>, CryptoError> {
    // serde_json maps NaN/inf to `null` silently; refuse instead so a broken
    // number can never be audited as "missing".
    record
        .serialize(finite::FiniteCheck)
        .map_err(|e| CryptoError::Serialize(e.0))?;
    // serde_json::Value's map is a BTreeMap (the `preserve_order` feature is
    // not enabled in this workspace), so keys serialize sorted at every level.
    let v = serde_json::to_value(record).map_err(|e| CryptoError::Serialize(e.to_string()))?;
    serde_json::to_vec(&v).map_err(|e| CryptoError::Serialize(e.to_string()))
}

/// A serializer that only walks the value and fails on non-finite floats.
mod finite {
    use serde::ser::{self, Serialize};

    #[derive(Debug)]
    pub struct Error(pub String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl std::error::Error for Error {}
    impl ser::Error for Error {
        fn custom<T: std::fmt::Display>(msg: T) -> Self {
            Error(msg.to_string())
        }
    }

    pub struct FiniteCheck;
    type R = Result<(), Error>;

    fn float(v: f64) -> R {
        if v.is_finite() {
            Ok(())
        } else {
            Err(Error(format!("non-finite float {v} in audit record")))
        }
    }

    macro_rules! ok_scalars {
        ($($m:ident: $t:ty),*) => { $(fn $m(self, _: $t) -> R { Ok(()) })* };
    }

    impl ser::Serializer for FiniteCheck {
        type Ok = ();
        type Error = Error;
        type SerializeSeq = Self;
        type SerializeTuple = Self;
        type SerializeTupleStruct = Self;
        type SerializeTupleVariant = Self;
        type SerializeMap = Self;
        type SerializeStruct = Self;
        type SerializeStructVariant = Self;
        ok_scalars!(serialize_bool: bool, serialize_i8: i8, serialize_i16: i16,
            serialize_i32: i32, serialize_i64: i64, serialize_i128: i128,
            serialize_u8: u8, serialize_u16: u16, serialize_u32: u32,
            serialize_u64: u64, serialize_u128: u128, serialize_char: char,
            serialize_str: &str, serialize_bytes: &[u8]);
        fn serialize_f32(self, v: f32) -> R {
            float(f64::from(v))
        }
        fn serialize_f64(self, v: f64) -> R {
            float(v)
        }
        fn serialize_none(self) -> R {
            Ok(())
        }
        fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> R {
            v.serialize(self)
        }
        fn serialize_unit(self) -> R {
            Ok(())
        }
        fn serialize_unit_struct(self, _: &'static str) -> R {
            Ok(())
        }
        fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> R {
            Ok(())
        }
        fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> R {
            v.serialize(self)
        }
        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            v: &T,
        ) -> R {
            v.serialize(self)
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_tuple(self, _: usize) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Error> {
            Ok(self)
        }
        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Self, Error> {
            Ok(self)
        }
    }

    macro_rules! compound {
        ($tr:ident, $($m:ident),*) => {
            impl ser::$tr for FiniteCheck {
                type Ok = ();
                type Error = Error;
                $(fn $m<T: ?Sized + Serialize>(&mut self, v: &T) -> R { v.serialize(FiniteCheck) })*
                fn end(self) -> R { Ok(()) }
            }
        };
    }
    compound!(SerializeSeq, serialize_element);
    compound!(SerializeTuple, serialize_element);
    compound!(SerializeTupleStruct, serialize_field);
    compound!(SerializeTupleVariant, serialize_field);

    impl ser::SerializeMap for FiniteCheck {
        type Ok = ();
        type Error = Error;
        fn serialize_key<T: ?Sized + Serialize>(&mut self, k: &T) -> R {
            k.serialize(FiniteCheck)
        }
        fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> R {
            v.serialize(FiniteCheck)
        }
        fn end(self) -> R {
            Ok(())
        }
    }
    impl ser::SerializeStruct for FiniteCheck {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &'static str, v: &T) -> R {
            v.serialize(FiniteCheck)
        }
        fn end(self) -> R {
            Ok(())
        }
    }
    impl ser::SerializeStructVariant for FiniteCheck {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &'static str, v: &T) -> R {
            v.serialize(FiniteCheck)
        }
        fn end(self) -> R {
            Ok(())
        }
    }
}

/// `h_n = BLAKE3(h_{n-1} || record)`.
pub fn chain_hash(prev: &[u8; 32], record: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(prev);
    h.update(record);
    *h.finalize().as_bytes()
}

/// One stored chain entry (`audit_log` row: n, record, hash, sig).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// 1-based position.
    pub n: u64,
    /// Decision the record belongs to.
    pub decision_id: String,
    /// `bytes(D_n)`, hex in JSON exports.
    #[serde(with = "hex_bytes")]
    pub record: Vec<u8>,
    /// `h_n`.
    #[serde(with = "hex_array")]
    pub hash: [u8; 32],
    /// `σ_n`.
    #[serde(with = "hex_array")]
    pub sig: [u8; 64],
}

/// A signed statement of the chain head, published out of band (for
/// example in each decision response) so truncation can be detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Number of records.
    pub n: u64,
    /// `h_n`.
    #[serde(with = "hex_array")]
    pub head: [u8; 32],
}

/// Appends records to a chain with the server key.
#[derive(Debug, Clone)]
pub struct AuditChain {
    n: u64,
    head: [u8; 32],
}

impl Default for AuditChain {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditChain {
    /// Empty chain at `h_0`.
    pub fn new() -> Self {
        Self {
            n: 0,
            head: GENESIS,
        }
    }

    /// Resumes after verified `entries` (e.g. on server restart).
    pub fn resume(entries: &[AuditEntry], key: &VerifyingKey) -> Result<Self, AuditFailure> {
        let report = verify_chain(entries, key, None);
        match report.failure {
            None => Ok(Self {
                n: report.verified,
                head: report.head,
            }),
            Some(f) => Err(f),
        }
    }

    /// Current checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            n: self.n,
            head: self.head,
        }
    }

    /// Appends `record` (already canonical bytes) and returns the entry to
    /// persist. The chain only advances on success.
    pub fn append(
        &mut self,
        server: &SigningIdentity,
        decision_id: &str,
        record: Vec<u8>,
    ) -> Result<AuditEntry, CryptoError> {
        let n = self
            .n
            .checked_add(1)
            .ok_or(CryptoError::SequenceExhausted)?;
        let hash = chain_hash(&self.head, &record);
        let sig = server.sign(Domain::Audit, &hash);
        self.n = n;
        self.head = hash;
        Ok(AuditEntry {
            n,
            decision_id: decision_id.to_string(),
            record,
            hash,
            sig,
        })
    }

    /// Serializes `record` canonically and appends it.
    pub fn append_record<T: Serialize>(
        &mut self,
        server: &SigningIdentity,
        decision_id: &str,
        record: &T,
    ) -> Result<AuditEntry, CryptoError> {
        let bytes = canonical_json(record)?;
        self.append(server, decision_id, bytes)
    }
}

/// What broke the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AuditFailure {
    /// Entry number is not the next expected one (missing or reordered).
    #[error("at position {position}: expected n={expected}, found n={found}")]
    Sequence {
        /// 1-based position in the input.
        position: u64,
        /// Expected `n`.
        expected: u64,
        /// Stored `n`.
        found: u64,
    },
    /// Recomputed hash differs from the stored one (record or hash modified,
    /// or an earlier record changed/removed).
    #[error("hash mismatch at n={n}")]
    HashMismatch {
        /// Entry number.
        n: u64,
    },
    /// Signature over `h_n` does not verify.
    #[error("bad signature at n={n}")]
    BadSignature {
        /// Entry number.
        n: u64,
    },
    /// Chain is shorter than the checkpoint (tail truncated).
    #[error("truncated: checkpoint says n={expected}, chain has {found}")]
    Truncated {
        /// Checkpoint length.
        expected: u64,
        /// Verified length.
        found: u64,
    },
    /// Chain head differs from the checkpoint at the checkpoint length.
    #[error("checkpoint head mismatch at n={n}")]
    CheckpointMismatch {
        /// Checkpoint length.
        n: u64,
    },
}

/// Result of [`verify_chain`]; mirrors `GET /api/audit/verify → {ok, broken_at}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// `true` when every entry verified (and the checkpoint matched, if given).
    pub ok: bool,
    /// Number of leading entries that verified.
    pub verified: u64,
    /// `n` (or position) of the first broken entry.
    pub broken_at: Option<u64>,
    /// Head after the last verified entry.
    #[serde(with = "hex_array")]
    pub head: [u8; 32],
    /// Failure detail.
    pub failure: Option<AuditFailure>,
}

/// Recomputes the chain from `h_0` and checks every entry. Stops at the
/// first failure. An empty chain verifies (`head = h_0`) unless a checkpoint
/// says it should be longer.
pub fn verify_chain(
    entries: &[AuditEntry],
    key: &VerifyingKey,
    checkpoint: Option<Checkpoint>,
) -> VerifyReport {
    let mut head = GENESIS;
    let mut verified = 0u64;
    let fail = |verified, head, broken_at, failure| VerifyReport {
        ok: false,
        verified,
        broken_at: Some(broken_at),
        head,
        failure: Some(failure),
    };
    for (i, e) in entries.iter().enumerate() {
        let expected = verified + 1;
        if e.n != expected {
            let position = i as u64 + 1;
            return fail(
                verified,
                head,
                position,
                AuditFailure::Sequence {
                    position,
                    expected,
                    found: e.n,
                },
            );
        }
        let h = chain_hash(&head, &e.record);
        if h != e.hash {
            return fail(verified, head, e.n, AuditFailure::HashMismatch { n: e.n });
        }
        if verify(key, Domain::Audit, &e.hash, &e.sig).is_err() {
            return fail(verified, head, e.n, AuditFailure::BadSignature { n: e.n });
        }
        head = h;
        verified = e.n;
    }
    if let Some(cp) = checkpoint {
        if verified < cp.n {
            return fail(
                verified,
                head,
                verified + 1,
                AuditFailure::Truncated {
                    expected: cp.n,
                    found: verified,
                },
            );
        }
        // Chain may legitimately have grown past the checkpoint; recompute
        // the head at the checkpoint position.
        let mut h = GENESIS;
        for e in entries.iter().take(cp.n as usize) {
            h = chain_hash(&h, &e.record);
        }
        if h != cp.head {
            return fail(
                verified,
                head,
                cp.n,
                AuditFailure::CheckpointMismatch { n: cp.n },
            );
        }
    }
    VerifyReport {
        ok: true,
        verified,
        broken_at: None,
        head,
        failure: None,
    }
}

/// Parses a JSON-lines export (one [`AuditEntry`] per line; blank lines
/// ignored).
pub fn parse_jsonl(text: &str) -> Result<Vec<AuditEntry>, CryptoError> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l)
                .map_err(|e| CryptoError::Serialize(format!("line {}: {e}", i + 1)))
        })
        .collect()
}

/// Serializes entries as JSON lines.
pub fn to_jsonl(entries: &[AuditEntry]) -> Result<String, CryptoError> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&serde_json::to_string(e).map_err(|e| CryptoError::Serialize(e.to_string()))?);
        out.push('\n');
    }
    Ok(out)
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

mod hex_array {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer, const N: usize>(b: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<[u8; N], D::Error> {
        let s = String::deserialize(d)?;
        let mut out = [0u8; N];
        hex::decode_to_slice(s, &mut out).map_err(serde::de::Error::custom)?;
        Ok(out)
    }
}
