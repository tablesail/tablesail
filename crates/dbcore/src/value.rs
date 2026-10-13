//! Typed cell values, shared by every engine.
//!
//! On the wire (Rust → UI) a `Value` is plain JSON where that is lossless, and a
//! one-key tagged object where it is not:
//!
//! | Value                     | JSON                                         |
//! |---------------------------|----------------------------------------------|
//! | Null                      | `null`                                       |
//! | Bool                      | `true` / `false`                             |
//! | Int/UInt within ±2^53     | number                                       |
//! | Int/UInt beyond ±2^53     | `{"$int": "9007199254740993"}`               |
//! | Float (finite)            | number                                       |
//! | Float (NaN/±Infinity)     | `{"$float": "NaN"}`                          |
//! | Decimal                   | `{"$dec": "12.3400"}`                        |
//! | Text                      | string                                       |
//! | Text (truncated)          | `{"$text": "first N chars", "len": total}`   |
//! | Bytes                     | `{"$bytes": "<base64 prefix>", "len": total}`|
//! | Json                      | `{"$json": <any JSON>}`                      |
//! | Date / Time / DateTime    | `{"$date": ".."}` / `{"$time": ".."}` / `{"$datetime": ".."}` |
//! | Uuid                      | `{"$uuid": ".."}`                            |
//! | ObjectId                  | `{"$oid": ".."}`                             |
//! | Other                     | `{"$other": "text", "type": "int4range"}`    |
//!
//! The TypeScript mirror lives in `src/lib/types.ts` (`Value`).

use base64::Engine as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

/// Largest integer JavaScript represents exactly.
pub const JS_SAFE_INT: i64 = 9_007_199_254_740_991;

/// Text longer than this is truncated when sent to the grid (full value via a cell fetch).
pub const GRID_TEXT_LIMIT: usize = 16 * 1024;
/// Bytes beyond this are not sent to the grid; `len` still reports the full size.
pub const GRID_BYTES_LIMIT: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    /// Exact numeric (NUMERIC, DECIMAL, money, Decimal128) as the server rendered it.
    Decimal(String),
    Text(String),
    /// Text cut to `GRID_TEXT_LIMIT` characters; `len` is the full length in characters.
    TruncatedText {
        prefix: String,
        len: usize,
    },
    /// Raw bytes; for grid pages only the first `GRID_BYTES_LIMIT` bytes are kept, `len` is the full size.
    Bytes {
        data: Vec<u8>,
        len: usize,
    },
    /// JSON/JSONB columns, Mongo sub-documents and arrays (relaxed Extended JSON).
    Json(serde_json::Value),
    /// Temporal values as the server rendered them, so zero dates, infinity and
    /// time zones survive untouched.
    Date(String),
    Time(String),
    DateTime(String),
    Uuid(String),
    ObjectId(String),
    /// Anything else (arrays, ranges, geometry, intervals, enums, MinKey…), as display text.
    Other {
        type_name: String,
        text: String,
    },
}

impl Value {
    pub fn bytes(data: Vec<u8>) -> Self {
        let len = data.len();
        Value::Bytes { data, len }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Returns a copy fit for a grid page: long text and large blobs are truncated.
    pub fn for_grid(self) -> Self {
        match self {
            Value::Text(s) if s.len() > GRID_TEXT_LIMIT => {
                let len = s.chars().count();
                if len <= GRID_TEXT_LIMIT {
                    return Value::Text(s);
                }
                Value::TruncatedText {
                    prefix: s.chars().take(GRID_TEXT_LIMIT).collect(),
                    len,
                }
            }
            Value::Bytes { mut data, len } if data.len() > GRID_BYTES_LIMIT => {
                data.truncate(GRID_BYTES_LIMIT);
                Value::Bytes { data, len }
            }
            other => other,
        }
    }

    /// True when this value is complete, i.e. safe to use in an optimistic-concurrency WHERE clause.
    pub fn is_complete(&self) -> bool {
        match self {
            Value::TruncatedText { .. } => false,
            Value::Bytes { data, len } => data.len() == *len,
            _ => true,
        }
    }

    /// Plain-text rendering, used by CSV export and the cell inspector.
    pub fn to_display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Decimal(s)
            | Value::Text(s)
            | Value::Date(s)
            | Value::Time(s)
            | Value::DateTime(s)
            | Value::Uuid(s)
            | Value::ObjectId(s) => s.clone(),
            Value::TruncatedText { prefix, .. } => prefix.clone(),
            Value::Bytes { data, .. } => format!("0x{}", hex::encode(data)),
            Value::Json(j) => j.to_string(),
            Value::Other { text, .. } => text.clone(),
        }
    }
}

fn tagged<S: Serializer>(s: S, tag: &str, v: &str) -> Result<S::Ok, S::Error> {
    let mut m = s.serialize_map(Some(1))?;
    m.serialize_entry(tag, v)?;
    m.end()
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(i) if (-JS_SAFE_INT..=JS_SAFE_INT).contains(i) => s.serialize_i64(*i),
            Value::Int(i) => tagged(s, "$int", &i.to_string()),
            Value::UInt(u) if *u <= JS_SAFE_INT as u64 => s.serialize_u64(*u),
            Value::UInt(u) => tagged(s, "$int", &u.to_string()),
            Value::Float(f) if f.is_finite() => s.serialize_f64(*f),
            Value::Float(f) => {
                let t = if f.is_nan() {
                    "NaN"
                } else if *f > 0.0 {
                    "Infinity"
                } else {
                    "-Infinity"
                };
                tagged(s, "$float", t)
            }
            Value::Decimal(d) => tagged(s, "$dec", d),
            Value::Text(t) => s.serialize_str(t),
            Value::TruncatedText { prefix, len } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("$text", prefix)?;
                m.serialize_entry("len", len)?;
                m.end()
            }
            Value::Bytes { data, len } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry(
                    "$bytes",
                    &base64::engine::general_purpose::STANDARD.encode(data),
                )?;
                m.serialize_entry("len", len)?;
                m.end()
            }
            Value::Json(j) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("$json", j)?;
                m.end()
            }
            Value::Date(d) => tagged(s, "$date", d),
            Value::Time(t) => tagged(s, "$time", t),
            Value::DateTime(d) => tagged(s, "$datetime", d),
            Value::Uuid(u) => tagged(s, "$uuid", u),
            Value::ObjectId(o) => tagged(s, "$oid", o),
            Value::Other { type_name, text } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("$other", text)?;
                m.serialize_entry("type", type_name)?;
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    /// Inverse of `Serialize`, so the UI can send back original values (e.g. primary keys).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        use serde_json::Value as J;
        let j = J::deserialize(d)?;
        Ok(match j {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(b),
            J::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::UInt(u)
                } else {
                    Value::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            J::String(s) => Value::Text(s),
            J::Array(_) => {
                return Err(D::Error::custom(
                    "arrays must be wrapped as {\"$json\": ...}",
                ));
            }
            J::Object(mut o) => {
                let str_of = |o: &mut serde_json::Map<String, J>, k: &str| -> Option<String> {
                    o.remove(k).and_then(|v| v.as_str().map(str::to_owned))
                };
                let len_of = |o: &serde_json::Map<String, J>| {
                    o.get("len").and_then(J::as_u64).unwrap_or(0) as usize
                };
                if let Some(v) = str_of(&mut o, "$int") {
                    v.parse::<i64>()
                        .map(Value::Int)
                        .or_else(|_| v.parse::<u64>().map(Value::UInt))
                        .map_err(D::Error::custom)?
                } else if let Some(v) = str_of(&mut o, "$float") {
                    Value::Float(v.parse::<f64>().map_err(D::Error::custom)?)
                } else if let Some(v) = str_of(&mut o, "$dec") {
                    Value::Decimal(v)
                } else if let Some(v) = str_of(&mut o, "$text") {
                    Value::TruncatedText {
                        len: len_of(&o),
                        prefix: v,
                    }
                } else if let Some(v) = str_of(&mut o, "$bytes") {
                    let data = base64::engine::general_purpose::STANDARD
                        .decode(v)
                        .map_err(D::Error::custom)?;
                    let len = o
                        .get("len")
                        .and_then(J::as_u64)
                        .map(|l| l as usize)
                        .unwrap_or(data.len());
                    Value::Bytes { data, len }
                } else if let Some(v) = o.remove("$json") {
                    Value::Json(v)
                } else if let Some(v) = str_of(&mut o, "$date") {
                    Value::Date(v)
                } else if let Some(v) = str_of(&mut o, "$time") {
                    Value::Time(v)
                } else if let Some(v) = str_of(&mut o, "$datetime") {
                    Value::DateTime(v)
                } else if let Some(v) = str_of(&mut o, "$uuid") {
                    Value::Uuid(v)
                } else if let Some(v) = str_of(&mut o, "$oid") {
                    Value::ObjectId(v)
                } else if let Some(v) = str_of(&mut o, "$other") {
                    Value::Other {
                        type_name: str_of(&mut o, "type").unwrap_or_default(),
                        text: v,
                    }
                } else {
                    return Err(D::Error::custom("unknown tagged value"));
                }
            }
        })
    }
}

/// A value typed by the user in the grid or a cell editor, sent UI → Rust.
///
/// JSON: `{"kind": "null"}`, `{"kind": "default"}`, `{"kind": "text", "value": "42"}`,
/// `{"kind": "bytes", "value": "<base64>"}`, `{"kind": "json", "value": {...}}`,
/// `{"kind": "original", "value": <Value>}` (unchanged, echo of what was loaded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CellInput {
    Null,
    /// The column's DEFAULT (inserts only).
    Default,
    /// Literal text; the server coerces it to the column type (SQL) or the driver
    /// converts it using the column's BSON type (Mongo).
    Text(String),
    /// Base64-encoded bytes.
    Bytes(String),
    /// A JSON document (JSON columns; Extended JSON for Mongo).
    Json(serde_json::Value),
    Original(Value),
}

/// One row of cells, in result-column order.
pub type Row = Vec<Value>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let values = vec![
            Value::Null,
            Value::Bool(true),
            Value::Int(42),
            Value::Int(9_007_199_254_740_993),
            Value::Float(f64::INFINITY),
            Value::Decimal("1.50".into()),
            Value::Text("hi".into()),
            Value::bytes(vec![0xde, 0xad]),
            Value::Json(serde_json::json!({"a": [1, 2]})),
            Value::DateTime("0000-00-00 00:00:00".into()),
            Value::ObjectId("65f000000000000000000001".into()),
            Value::Other {
                type_name: "int4range".into(),
                text: "[1,10)".into(),
            },
        ];
        for v in values {
            let json = serde_json::to_string(&v).unwrap();
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back, v, "{json}");
        }
    }

    #[test]
    fn big_ints_are_strings() {
        assert_eq!(
            serde_json::to_string(&Value::Int(9_007_199_254_740_993)).unwrap(),
            r#"{"$int":"9007199254740993"}"#
        );
        assert_eq!(serde_json::to_string(&Value::Int(42)).unwrap(), "42");
    }
}
