//! Minimal RFC 8949 codec covering what nlohmann::json::to_cbor emits.
//!
//! Floats keep their encoded width so a decode/encode round trip is byte-exact,
//! and maps keep their encoded order (nlohmann emits std::map order, i.e. bytewise).

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Covers the full CBOR integer range (major types 0 and 1).
    Int(i128),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    /// Entries in encoded order.
    Map(Vec<(Value, Value)>),
    Tag(u64, Box<Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find_map(|(k, v)| match k {
                Value::Text(s) if s == key => Some(v),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Numeric value regardless of integer/float encoding.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Int(i) => Some(i as f64),
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(Value, Value)] {
        match self {
            Value::Map(entries) => entries,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub offset: usize,
    pub reason: &'static str,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CBOR {} at byte {}", self.reason, self.offset)
    }
}

impl std::error::Error for DecodeError {}

/// Decodes exactly one CBOR item spanning the whole input.
pub fn decode(bytes: &[u8]) -> Result<Value, DecodeError> {
    let mut reader = Reader { bytes, pos: 0, depth: 0 };
    let value = reader.read()?;
    if reader.pos != bytes.len() {
        return Err(reader.error("trailing bytes"));
    }
    Ok(value)
}

// Deep enough for any preset; bounds recursion on hostile input.
const MAX_DEPTH: usize = 256;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn error(&self, reason: &'static str) -> DecodeError {
        DecodeError { offset: self.pos, reason }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or_else(|| self.error("unexpected end"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn argument(&mut self, info: u8) -> Result<u64, DecodeError> {
        Ok(match info {
            0..=23 => info as u64,
            24 => self.take(1)?[0] as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            _ => return Err(self.error("unsupported length encoding")),
        })
    }

    /// Lengths are checked against the remaining input before allocating.
    fn length(&mut self, info: u8, min_item_bytes: usize) -> Result<usize, DecodeError> {
        let n = self.argument(info)?;
        let remaining = (self.bytes.len() - self.pos) as u64;
        if n.saturating_mul(min_item_bytes as u64) > remaining {
            return Err(self.error("length exceeds input"));
        }
        Ok(n as usize)
    }

    fn read(&mut self) -> Result<Value, DecodeError> {
        let initial = self.take(1)?[0];
        let (major, info) = (initial >> 5, initial & 0x1f);
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        let value = match major {
            0 => Value::Int(self.argument(info)? as i128),
            1 => Value::Int(-1 - self.argument(info)? as i128),
            2 => {
                let n = self.length(info, 1)?;
                Value::Bytes(self.take(n)?.to_vec())
            }
            3 => {
                let n = self.length(info, 1)?;
                let raw = self.take(n)?;
                Value::Text(String::from_utf8(raw.to_vec()).map_err(|_| self.error("invalid UTF-8 text"))?)
            }
            4 => {
                let n = self.length(info, 1)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.read()?);
                }
                Value::Array(items)
            }
            5 => {
                let n = self.length(info, 2)?;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = self.read()?;
                    entries.push((k, self.read()?));
                }
                Value::Map(entries)
            }
            6 => {
                let tag = self.argument(info)?;
                Value::Tag(tag, Box::new(self.read()?))
            }
            _ => match info {
                20 => Value::Bool(false),
                21 => Value::Bool(true),
                22 => Value::Null,
                25 => Value::Float(half_to_f64(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))),
                26 => Value::Float(f32::from_be_bytes(self.take(4)?.try_into().unwrap()) as f64),
                27 => Value::Float(f64::from_be_bytes(self.take(8)?.try_into().unwrap())),
                _ => return Err(self.error("unsupported simple value")),
            },
        };
        self.depth -= 1;
        Ok(value)
    }
}

fn half_to_f64(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as f64;
    match exp {
        0 => sign * frac * 2f64.powi(-24),
        31 if frac == 0.0 => sign * f64::INFINITY,
        31 => f64::NAN,
        _ => sign * (1.0 + frac / 1024.0) * 2f64.powi(exp - 15),
    }
}

/// Encodes with nlohmann's choices: shortest integer heads, float32 when exact, else float64.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, value);
    out
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend([m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend(n.to_be_bytes());
        }
    }
}

fn write(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.push(0xf6),
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Int(i) if *i >= 0 => head(out, 0, *i as u64),
        Value::Int(i) => head(out, 1, (-1 - *i) as u64),
        Value::Float(f) => {
            let narrow = *f as f32;
            if narrow as f64 == *f || f.is_nan() {
                out.push(0xfa);
                out.extend(narrow.to_be_bytes());
            } else {
                out.push(0xfb);
                out.extend(f.to_be_bytes());
            }
        }
        Value::Text(s) => {
            head(out, 3, s.len() as u64);
            out.extend(s.as_bytes());
        }
        Value::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend(b);
        }
        Value::Array(items) => {
            head(out, 4, items.len() as u64);
            items.iter().for_each(|v| write(out, v));
        }
        Value::Map(entries) => {
            head(out, 5, entries.len() as u64);
            for (k, v) in entries {
                write(out, k);
                write(out, v);
            }
        }
        Value::Tag(tag, inner) => {
            head(out, 6, *tag);
            write(out, inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_floats_keep_float_encoding() {
        let v = Value::Map(vec![(Value::Text("a".into()), Value::Float(1.0)), (Value::Text("b".into()), Value::Int(1))]);
        let bytes = encode(&v);
        assert_eq!(bytes, [0xa2, 0x61, b'a', 0xfa, 0x3f, 0x80, 0, 0, 0x61, b'b', 0x01]);
        assert_eq!(decode(&bytes).unwrap(), v);
    }

    #[test]
    fn inexact_floats_use_double() {
        let bytes = encode(&Value::Float(0.1));
        assert_eq!(bytes[0], 0xfb);
    }

    #[test]
    fn rejects_lengths_beyond_input() {
        // Array claiming 2^32 items with no content must fail before allocating.
        assert!(decode(&[0x9a, 0xff, 0xff, 0xff, 0xff]).is_err());
    }
}
