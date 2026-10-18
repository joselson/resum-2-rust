//! The XferJson container used by `.SerumPreset` files.
//!
//! ```text
//! 0      char[9]  "XferJson\0"
//! 9      u64 LE   metadata JSON length N
//! 17     char[N]  metadata JSON; "hash" = lowercase hex MD5 of the zstd frame
//! 17+N   u32 LE   uncompressed CBOR size (> 0)
//! 21+N   u32 LE   container version (2 is written; < 3 is accepted)
//! 25+N   ...      zstd frame (level 3) of the CBOR payload
//! ```

pub mod cbor;

use md5::{Digest, Md5};
use std::fmt;

pub use cbor::Value;

const MAGIC: &[u8; 9] = b"XferJson\0";
const HEADER_LEN: usize = 17;
pub const CONTAINER_VERSION: u32 = 2;

#[derive(Debug)]
pub enum Error {
    NotXferJson,
    Truncated(&'static str),
    Metadata(serde_json::Error),
    UnsupportedVersion(u32),
    Decompress(std::io::Error),
    SizeMismatch { header: u32, actual: usize },
    Cbor(cbor::DecodeError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotXferJson => write!(f, "not an XferJson container"),
            Error::Truncated(what) => write!(f, "truncated {what}"),
            Error::Metadata(e) => write!(f, "invalid metadata JSON: {e}"),
            Error::UnsupportedVersion(v) => write!(f, "unsupported container version {v}"),
            Error::Decompress(e) => write!(f, "zstd: {e}"),
            Error::SizeMismatch { header, actual } => write!(f, "payload size mismatch: header {header}, decompressed {actual}"),
            Error::Cbor(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub struct Preset {
    /// Metadata with its original key order. `hash` is recomputed on write.
    pub metadata: serde_json::Map<String, serde_json::Value>,
    pub version: u32,
    /// Whether the stored hash matched the frame when read.
    pub hash_valid: bool,
    pub payload: Value,
}

impl Preset {
    pub fn read(bytes: &[u8]) -> Result<Preset, Error> {
        if bytes.len() < HEADER_LEN || &bytes[..9] != MAGIC {
            return Err(Error::NotXferJson);
        }
        let meta_len = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
        let meta_end = usize::try_from(meta_len)
            .ok()
            .and_then(|n| n.checked_add(HEADER_LEN))
            .filter(|&end| end.checked_add(8).is_some_and(|e| e <= bytes.len()))
            .ok_or(Error::Truncated("metadata"))?;
        let metadata: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&bytes[HEADER_LEN..meta_end]).map_err(Error::Metadata)?;
        let cbor_size = u32::from_le_bytes(bytes[meta_end..meta_end + 4].try_into().unwrap());
        let version = u32::from_le_bytes(bytes[meta_end + 4..meta_end + 8].try_into().unwrap());
        if version >= 3 {
            return Err(Error::UnsupportedVersion(version));
        }
        let frame = &bytes[meta_end + 8..];
        let hash_valid = metadata.get("hash").and_then(|h| h.as_str()) == Some(md5_hex(frame).as_str());
        // The header bounds the allocation; a frame that expands further is rejected by zstd.
        let cbor = zstd::bulk::decompress(frame, cbor_size as usize).map_err(Error::Decompress)?;
        if cbor.len() != cbor_size as usize {
            return Err(Error::SizeMismatch { header: cbor_size, actual: cbor.len() });
        }
        let payload = cbor::decode(&cbor).map_err(Error::Cbor)?;
        Ok(Preset { metadata, version, hash_valid, payload })
    }

    pub fn write(&self) -> Vec<u8> {
        let cbor = cbor::encode(&self.payload);
        let frame = zstd::bulk::compress(&cbor, 3).expect("in-memory zstd compression");
        let mut metadata = self.metadata.clone();
        metadata.insert("hash".into(), md5_hex(&frame).into());
        let meta = serde_json::to_vec(&metadata).expect("JSON map serializes");
        let mut out = Vec::with_capacity(HEADER_LEN + meta.len() + 8 + frame.len());
        out.extend(MAGIC);
        out.extend((meta.len() as u64).to_le_bytes());
        out.extend(&meta);
        out.extend((cbor.len() as u32).to_le_bytes());
        out.extend(self.version.to_le_bytes());
        out.extend(&frame);
        out
    }
}

fn md5_hex(bytes: &[u8]) -> String {
    Md5::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Preset {
        let mut metadata = serde_json::Map::new();
        metadata.insert("fileType".into(), "SerumPreset".into());
        let payload = Value::Map(vec![(Value::Text("Env0".into()), Value::Map(vec![(Value::Text("kParamSustain".into()), Value::Float(0.5))]))]);
        Preset { metadata, version: CONTAINER_VERSION, hash_valid: true, payload }
    }

    #[test]
    fn round_trips_and_validates_hash() {
        let bytes = sample().write();
        let back = Preset::read(&bytes).unwrap();
        assert!(back.hash_valid);
        assert_eq!(back.payload, sample().payload);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert!(matches!(Preset::read(b"NotXfer"), Err(Error::NotXferJson)));
        let bytes = sample().write();
        assert!(Preset::read(&bytes[..20]).is_err());
    }

    /// Byte-exact round trip over real presets: `LUMEN_PRESET_DIR=... cargo test -p lumen-preset`.
    #[test]
    fn factory_presets_round_trip() {
        let Some(dir) = std::env::var_os("LUMEN_PRESET_DIR") else { return };
        let mut stack = vec![std::path::PathBuf::from(dir)];
        let mut count = 0;
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("SerumPreset")) {
                let bytes = std::fs::read(&path).unwrap();
                let preset = Preset::read(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(preset.hash_valid, "{}", path.display());
                let cbor_offset = HEADER_LEN + u64::from_le_bytes(bytes[9..17].try_into().unwrap()) as usize + 8;
                let original = zstd::stream::decode_all(&bytes[cbor_offset..]).unwrap();
                assert_eq!(cbor::encode(&preset.payload), original, "{}", path.display());
                count += 1;
            }
        }
        assert!(count > 0, "no presets found");
        eprintln!("verified {count} presets");
    }
}
