//! BSON <-> `Value`, Extended JSON helpers, date parsing and error mapping.
//! Everything here is typed end to end: ObjectId, Decimal128, Int64, Date and
//! Binary never degrade to strings or doubles on the way out or back in.

use base64::Engine as _;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Binary, Bson, DateTime, Decimal128, Document, oid::ObjectId};
use serde_json::Value as Json;

use crate::error::{Error, Result};
use crate::model::{ColumnInfo, TypeFamily};
use crate::value::{CellInput, Value};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Server error code of a driver error, when it carries one.
pub fn error_code(err: &mongodb::error::Error) -> Option<i32> {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*err.kind {
        ErrorKind::Command(c) => Some(c.code),
        ErrorKind::Write(WriteFailure::WriteError(w)) => Some(w.code),
        ErrorKind::Write(WriteFailure::WriteConcernError(w)) => Some(w.code),
        ErrorKind::InsertMany(im) => im
            .write_errors
            .as_ref()
            .and_then(|e| e.first())
            .map(|e| e.code),
        _ => None,
    }
}

/// Maps a driver error to ours, keeping the server's own message and code.
pub fn db_err(err: mongodb::error::Error) -> Error {
    use mongodb::error::{ErrorKind, WriteFailure};
    let database = |message: String, code: i32| Error::Database {
        message,
        code: Some(code.to_string()),
        statement: None,
    };
    match &*err.kind {
        ErrorKind::Command(c) => database(c.message.clone(), c.code),
        ErrorKind::Write(WriteFailure::WriteError(w)) => database(w.message.clone(), w.code),
        ErrorKind::Write(WriteFailure::WriteConcernError(w)) => database(w.message.clone(), w.code),
        ErrorKind::InsertMany(im) => match im.write_errors.as_ref().and_then(|e| e.first()) {
            Some(e) => database(e.message.clone(), e.code),
            None => Error::database(err.to_string()),
        },
        ErrorKind::ServerSelection { message, .. } => Error::Connection(message.clone()),
        ErrorKind::Authentication { message, .. } => {
            Error::Connection(format!("authentication failed: {message}"))
        }
        ErrorKind::Io(_)
        | ErrorKind::ConnectionPoolCleared { .. }
        | ErrorKind::DnsResolve { .. }
        | ErrorKind::InvalidTlsConfig { .. } => Error::Connection(err.to_string()),
        ErrorKind::InvalidArgument { message, .. } => Error::InvalidInput(message.clone()),
        _ => Error::database(err.to_string()),
    }
}

/// True for the error a standalone server returns when asked to run a transaction.
pub fn is_transactions_unsupported(err: &Error) -> bool {
    match err {
        Error::Database { message, code, .. } => {
            code.as_deref() == Some("20")
                && (message.contains("Transaction numbers")
                    || message.contains("replica set")
                    || message.contains("Transactions are not supported"))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

const MILLIS_PER_DAY: i64 = 86_400_000;
/// 0000-01-01T00:00:00Z and 9999-12-31T23:59:59.999Z.
const MIN_MILLIS: i64 = -62_167_219_200_000;
const MAX_MILLIS: i64 = 253_402_300_799_999;

/// RFC 3339 with millisecond precision, or None outside years 0000..=9999.
pub fn format_millis(ms: i64) -> Option<String> {
    if !(MIN_MILLIS..=MAX_MILLIS).contains(&ms) {
        return None;
    }
    let days = ms.div_euclid(MILLIS_PER_DAY);
    let rem = ms.rem_euclid(MILLIS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s, milli) = (
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000,
    );
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z"
    ))
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Parses user-typed dates into epoch millis: RFC 3339, `YYYY-MM-DD HH:MM[:SS[.fff]]`
/// (UTC unless an offset is given) and plain `YYYY-MM-DD`.
pub fn parse_datetime(text: &str) -> Option<i64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let (date, time) = match s.find(['T', 't', ' ']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let bytes = date.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shaped {
        return None;
    }
    let mut normalized = String::with_capacity(s.len() + 10);
    normalized.push_str(date);
    normalized.push('T');
    match time {
        None => normalized.push_str("00:00:00Z"),
        Some(t) => {
            let t = t.trim();
            // Split the clock part from a trailing zone designator.
            let zone_at = t.find(['Z', 'z', '+', '-']);
            let (clock, zone) = match zone_at {
                Some(i) => (&t[..i], t[i..].trim()),
                None => (t, ""),
            };
            normalized.push_str(clock.trim());
            if clock.matches(':').count() == 1 {
                normalized.push_str(":00");
            }
            normalized.push_str(if zone.is_empty() { "Z" } else { zone });
        }
    }
    DateTime::parse_rfc3339_str(&normalized)
        .ok()
        .map(|d| d.timestamp_millis())
}

// ---------------------------------------------------------------------------
// BSON -> Value
// ---------------------------------------------------------------------------

/// A field missing from a document: distinct from an explicit null.
pub fn missing() -> Value {
    Value::Other {
        type_name: "missing".into(),
        text: String::new(),
    }
}

pub fn is_missing(value: &Value) -> bool {
    matches!(value, Value::Other { type_name, .. } if type_name == "missing")
}

fn other(type_name: &str, text: String) -> Value {
    Value::Other {
        type_name: type_name.into(),
        text,
    }
}

pub fn bson_to_value(bson: &Bson) -> Value {
    match bson {
        Bson::Double(f) => Value::Float(*f),
        Bson::String(s) => Value::Text(s.clone()),
        Bson::Boolean(b) => Value::Bool(*b),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Int32(i) => Value::Int(i64::from(*i)),
        Bson::Int64(i) => Value::Int(*i),
        Bson::ObjectId(o) => Value::ObjectId(o.to_hex()),
        Bson::DateTime(d) => match format_millis(d.timestamp_millis()) {
            Some(s) => Value::DateTime(s),
            None => other("date", d.timestamp_millis().to_string()),
        },
        Bson::Decimal128(d) => Value::Decimal(d.to_string()),
        Bson::Binary(bin) => match (bin.subtype, bin.bytes.len()) {
            (BinarySubtype::Uuid | BinarySubtype::UuidOld, 16) => {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&bin.bytes);
                Value::Uuid(uuid::Uuid::from_bytes(raw).hyphenated().to_string())
            }
            _ => Value::bytes(bin.bytes.clone()),
        },
        Bson::Document(_) | Bson::Array(_) => Value::Json(bson_to_json(bson)),
        Bson::Timestamp(t) => other(
            "timestamp",
            format!("Timestamp({}, {})", t.time, t.increment),
        ),
        Bson::RegularExpression(r) => other("regex", format!("/{}/{}", r.pattern, r.options)),
        Bson::JavaScriptCode(c) => other("javascript", c.clone()),
        Bson::JavaScriptCodeWithScope(c) => other("javascriptWithScope", c.code.clone()),
        Bson::Symbol(s) => other("symbol", s.clone()),
        Bson::MinKey => other("minKey", "MinKey".into()),
        Bson::MaxKey => other("maxKey", "MaxKey".into()),
        Bson::DbPointer(_) => other("dbPointer", bson.to_string()),
    }
}

/// Relaxed Extended JSON of a BSON value, built from a reference so large
/// documents are not cloned first.
pub fn bson_to_json(bson: &Bson) -> Json {
    match bson {
        Bson::Document(d) => Json::Object(
            d.iter()
                .map(|(k, v)| (k.clone(), bson_to_json(v)))
                .collect(),
        ),
        Bson::Array(a) => Json::Array(a.iter().map(bson_to_json).collect()),
        leaf => leaf.clone().into_relaxed_extjson(),
    }
}

/// Short BSON type name, as `$type` aliases it.
pub fn type_name(bson: &Bson) -> &'static str {
    match bson {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Document(_) => "object",
        Bson::Array(_) => "array",
        Bson::Binary(b) if is_uuid(b) => "uuid",
        Bson::Binary(_) => "binData",
        Bson::Undefined => "undefined",
        Bson::ObjectId(_) => "objectId",
        Bson::Boolean(_) => "bool",
        Bson::DateTime(_) => "date",
        Bson::Null => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::DbPointer(_) => "dbPointer",
        Bson::JavaScriptCode(_) => "javascript",
        Bson::Symbol(_) => "symbol",
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope",
        Bson::Int32(_) => "int",
        Bson::Timestamp(_) => "timestamp",
        Bson::Int64(_) => "long",
        Bson::Decimal128(_) => "decimal",
        Bson::MinKey => "minKey",
        Bson::MaxKey => "maxKey",
    }
}

fn is_uuid(b: &Binary) -> bool {
    matches!(b.subtype, BinarySubtype::Uuid | BinarySubtype::UuidOld) && b.bytes.len() == 16
}

pub fn family_of(bson: &Bson) -> TypeFamily {
    match bson {
        Bson::ObjectId(_) => TypeFamily::ObjectId,
        Bson::String(_) | Bson::Symbol(_) | Bson::JavaScriptCode(_) => TypeFamily::Text,
        Bson::Int32(_) | Bson::Int64(_) => TypeFamily::Integer,
        Bson::Double(_) => TypeFamily::Float,
        Bson::Decimal128(_) => TypeFamily::Decimal,
        Bson::Boolean(_) => TypeFamily::Boolean,
        Bson::DateTime(_) => TypeFamily::DateTime,
        Bson::Binary(b) if is_uuid(b) => TypeFamily::Uuid,
        Bson::Binary(_) => TypeFamily::Binary,
        Bson::Document(_) => TypeFamily::Document,
        Bson::Array(_) => TypeFamily::Array,
        _ => TypeFamily::Other,
    }
}

// ---------------------------------------------------------------------------
// Extended JSON -> BSON
// ---------------------------------------------------------------------------

const EXTENDED_KEYS: [&str; 16] = [
    "$oid",
    "$date",
    "$numberInt",
    "$numberLong",
    "$numberDouble",
    "$numberDecimal",
    "$binary",
    "$regularExpression",
    "$timestamp",
    "$minKey",
    "$maxKey",
    "$undefined",
    "$symbol",
    "$code",
    "$dbPointer",
    "$numberDecimalBytes",
];

/// Converts (relaxed or canonical) Extended JSON to BSON. Plain JSON integers
/// become Int32 when they fit and Int64 otherwise; fractional numbers Double.
pub fn json_to_bson(json: Json) -> Result<Bson> {
    Ok(match json {
        Json::Null => Bson::Null,
        Json::Bool(b) => Bson::Boolean(b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i32::try_from(i) {
                    Ok(small) => Bson::Int32(small),
                    Err(_) => Bson::Int64(i),
                }
            } else {
                Bson::Double(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Json::String(s) => Bson::String(s),
        Json::Array(items) => Bson::Array(
            items
                .into_iter()
                .map(json_to_bson)
                .collect::<Result<Vec<_>>>()?,
        ),
        Json::Object(map) => {
            let extended = map.len() <= 2
                && map
                    .keys()
                    .next()
                    .is_some_and(|k| EXTENDED_KEYS.contains(&k.as_str()));
            if extended {
                serde_json::from_value::<Bson>(Json::Object(map))
                    .map_err(|e| Error::InvalidInput(format!("Invalid Extended JSON: {e}")))?
            } else {
                let mut doc = Document::new();
                for (k, v) in map {
                    doc.insert(k, json_to_bson(v)?);
                }
                Bson::Document(doc)
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Value / CellInput -> BSON
// ---------------------------------------------------------------------------

/// Converts a value loaded from the grid back to BSON (primary keys, optimistic
/// checks). Truncated values are refused: they are not the stored value.
pub fn value_to_bson(value: &Value) -> Result<Bson> {
    Ok(match value {
        Value::Null => Bson::Null,
        Value::Bool(b) => Bson::Boolean(*b),
        Value::Int(i) => match i32::try_from(*i) {
            Ok(small) => Bson::Int32(small),
            Err(_) => Bson::Int64(*i),
        },
        Value::UInt(u) => match i64::try_from(*u) {
            Ok(i) => Bson::Int64(i),
            Err(_) => Bson::Double(*u as f64),
        },
        Value::Float(f) => Bson::Double(*f),
        Value::Decimal(s) => Bson::Decimal128(
            s.parse::<Decimal128>()
                .map_err(|e| Error::InvalidInput(format!("Invalid decimal {s:?}: {e}")))?,
        ),
        Value::Text(s) => Bson::String(s.clone()),
        Value::TruncatedText { .. } => {
            return Err(Error::InvalidInput(
                "Value is truncated and cannot be used as a key".into(),
            ));
        }
        Value::Bytes { data, len } => {
            if data.len() != *len {
                return Err(Error::InvalidInput(
                    "Binary value is truncated and cannot be used as a key".into(),
                ));
            }
            Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: data.clone(),
            })
        }
        Value::Json(j) => json_to_bson(j.clone())?,
        Value::Date(s) | Value::DateTime(s) => {
            let ms = parse_datetime(s)
                .ok_or_else(|| Error::InvalidInput(format!("Invalid date {s:?}")))?;
            Bson::DateTime(DateTime::from_millis(ms))
        }
        Value::Time(s) => Bson::String(s.clone()),
        Value::Uuid(s) => uuid_bson(s)?,
        Value::ObjectId(s) => Bson::ObjectId(
            ObjectId::parse_str(s)
                .map_err(|e| Error::InvalidInput(format!("Invalid ObjectId {s:?}: {e}")))?,
        ),
        Value::Other { type_name, text } => match type_name.as_str() {
            "date" => text
                .parse::<i64>()
                .map(|ms| Bson::DateTime(DateTime::from_millis(ms)))
                .map_err(|_| Error::InvalidInput(format!("Invalid date {text:?}")))?,
            "minKey" => Bson::MinKey,
            "maxKey" => Bson::MaxKey,
            _ => {
                return Err(Error::InvalidInput(format!(
                    "Cannot use a {type_name} value here"
                )));
            }
        },
    })
}

fn uuid_bson(text: &str) -> Result<Bson> {
    let parsed = uuid::Uuid::parse_str(text.trim())
        .map_err(|e| Error::InvalidInput(format!("Invalid UUID {text:?}: {e}")))?;
    Ok(Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes: parsed.as_bytes().to_vec(),
    }))
}

/// The BSON type a typed-in cell should become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Unknown,
    Int32,
    Int64,
    Double,
    Decimal,
    Bool,
    DateTime,
    ObjectId,
    Uuid,
    Str,
    Binary,
    /// Document or array: the text must be JSON.
    Container,
}

impl Target {
    /// From the field's current value when known, else from the inferred column.
    pub fn resolve(original: Option<&Value>, column: Option<&ColumnInfo>) -> Target {
        let from_original = match original {
            Some(Value::Int(i)) => Some(match column {
                Some(c) if c.data_type == "long" => Target::Int64,
                Some(c) if c.data_type == "int" => Target::Int32,
                _ if i32::try_from(*i).is_ok() => Target::Int32,
                _ => Target::Int64,
            }),
            Some(Value::UInt(_)) => Some(Target::Int64),
            Some(Value::Float(_)) => Some(Target::Double),
            Some(Value::Decimal(_)) => Some(Target::Decimal),
            Some(Value::Bool(_)) => Some(Target::Bool),
            Some(Value::Text(_) | Value::TruncatedText { .. }) => Some(Target::Str),
            Some(Value::Date(_) | Value::DateTime(_)) => Some(Target::DateTime),
            Some(Value::ObjectId(_)) => Some(Target::ObjectId),
            Some(Value::Uuid(_)) => Some(Target::Uuid),
            Some(Value::Bytes { .. }) => Some(Target::Binary),
            Some(Value::Json(_)) => Some(Target::Container),
            _ => None,
        };
        from_original
            .or_else(|| column.map(Target::from_column))
            .unwrap_or(Target::Unknown)
    }

    pub fn from_column(column: &ColumnInfo) -> Target {
        match column.family {
            TypeFamily::Integer => {
                if column.data_type == "long" {
                    Target::Int64
                } else {
                    Target::Int32
                }
            }
            TypeFamily::Float => Target::Double,
            TypeFamily::Decimal => Target::Decimal,
            TypeFamily::Boolean => Target::Bool,
            TypeFamily::Text => Target::Str,
            TypeFamily::Binary => Target::Binary,
            TypeFamily::Uuid => Target::Uuid,
            TypeFamily::ObjectId => Target::ObjectId,
            TypeFamily::Date | TypeFamily::DateTime => Target::DateTime,
            TypeFamily::Document | TypeFamily::Array | TypeFamily::Json => Target::Container,
            _ => Target::Unknown,
        }
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" | "y" => Some(true),
        "false" | "f" | "0" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_f64(text: &str) -> Option<f64> {
    match text.trim() {
        "NaN" => Some(f64::NAN),
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        t => t.parse::<f64>().ok().filter(|f| f.is_finite()),
    }
}

/// Parses typed-in text into the target type. Text that does not parse stays a
/// string (Mongo is schemaless), except for document/array targets where
/// silently turning a sub-document into a string would destroy data.
pub fn text_to_bson(text: &str, target: Target) -> Result<Bson> {
    let trimmed = text.trim();
    let parsed = match target {
        Target::Int32 | Target::Int64 => trimmed.parse::<i64>().ok().map(|i| {
            if target == Target::Int32 {
                match i32::try_from(i) {
                    Ok(small) => Bson::Int32(small),
                    Err(_) => Bson::Int64(i),
                }
            } else {
                Bson::Int64(i)
            }
        }),
        Target::Double => parse_f64(trimmed).map(Bson::Double),
        Target::Decimal => trimmed.parse::<Decimal128>().ok().map(Bson::Decimal128),
        Target::Bool => parse_bool(trimmed).map(Bson::Boolean),
        Target::DateTime => {
            parse_datetime(trimmed).map(|ms| Bson::DateTime(DateTime::from_millis(ms)))
        }
        Target::ObjectId => ObjectId::parse_str(trimmed).ok().map(Bson::ObjectId),
        Target::Uuid => uuid_bson(trimmed).ok(),
        Target::Binary => hex_or_base64(trimmed).map(|bytes| {
            Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes,
            })
        }),
        Target::Container => {
            return super::shell::parse_value(text).and_then(|b| match b {
                Bson::Document(_) | Bson::Array(_) => Ok(b),
                _ => Err(Error::InvalidInput(
                    "This field holds a document or array: enter valid JSON".into(),
                )),
            });
        }
        Target::Str | Target::Unknown => None,
    };
    Ok(parsed.unwrap_or_else(|| Bson::String(text.to_owned())))
}

fn hex_or_base64(text: &str) -> Option<Vec<u8>> {
    if let Some(h) = text.strip_prefix("0x") {
        return hex::decode(h).ok();
    }
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// A cell edit as a BSON value; `None` means "no value" (CellInput::Default).
pub fn cell_to_bson(input: &CellInput, target: Target) -> Result<Option<Bson>> {
    Ok(Some(match input {
        CellInput::Default => return Ok(None),
        CellInput::Null => Bson::Null,
        CellInput::Text(t) => text_to_bson(t, target)?,
        CellInput::Bytes(b64) => Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| Error::InvalidInput(format!("Invalid base64: {e}")))?,
        }),
        CellInput::Json(j) => json_to_bson(j.clone())?,
        CellInput::Original(v) => value_to_bson(v)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn formats_dates_with_millis() {
        assert_eq!(
            format_millis(1_714_566_896_789).as_deref(),
            Some("2024-05-01T12:34:56.789Z")
        );
        assert_eq!(
            format_millis(0).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
        assert_eq!(
            format_millis(-1).as_deref(),
            Some("1969-12-31T23:59:59.999Z")
        );
        assert_eq!(format_millis(MAX_MILLIS + 1), None);
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_datetime("2024-05-01"), Some(1_714_521_600_000));
        assert_eq!(
            parse_datetime("2024-05-01 12:34:56.789"),
            Some(1_714_566_896_789)
        );
        assert_eq!(parse_datetime("2024-05-01T12:34"), Some(1_714_566_840_000));
        assert_eq!(
            parse_datetime("2024-05-01T14:34:56.789+02:00"),
            Some(1_714_566_896_789)
        );
        assert_eq!(parse_datetime("nope"), None);
    }

    #[test]
    fn bson_values_keep_their_type() {
        let oid = ObjectId::parse_str("65f000000000000000000001").unwrap();
        assert_eq!(
            bson_to_value(&Bson::ObjectId(oid)),
            Value::ObjectId("65f000000000000000000001".into())
        );
        assert_eq!(bson_to_value(&Bson::Int64(7)), Value::Int(7));
        assert_eq!(
            bson_to_value(&Bson::Decimal128("1.50".parse().unwrap())),
            Value::Decimal("1.50".into())
        );
        let uuid = Binary {
            subtype: BinarySubtype::Uuid,
            bytes: (1u8..=16).collect(),
        };
        assert_eq!(
            bson_to_value(&Bson::Binary(uuid)),
            Value::Uuid("01020304-0506-0708-090a-0b0c0d0e0f10".into())
        );
        assert!(matches!(
            bson_to_value(&Bson::DateTime(DateTime::from_millis(i64::MAX))),
            Value::Other { .. }
        ));
        assert!(matches!(
            bson_to_value(&Bson::Document(doc! {"a": 1})),
            Value::Json(_)
        ));
    }

    #[test]
    fn json_numbers_pick_int_width() {
        assert_eq!(json_to_bson(serde_json::json!(5)).unwrap(), Bson::Int32(5));
        assert_eq!(
            json_to_bson(serde_json::json!(-5)).unwrap(),
            Bson::Int32(-5)
        );
        assert_eq!(
            json_to_bson(serde_json::json!(5_000_000_000i64)).unwrap(),
            Bson::Int64(5_000_000_000)
        );
        assert_eq!(
            json_to_bson(serde_json::json!(1.5)).unwrap(),
            Bson::Double(1.5)
        );
        assert!(matches!(
            json_to_bson(serde_json::json!({"$oid": "65f000000000000000000001"})).unwrap(),
            Bson::ObjectId(_)
        ));
        let regex_doc = json_to_bson(serde_json::json!({"$regex": "a", "$options": "i"})).unwrap();
        assert!(matches!(regex_doc, Bson::Document(_)));
    }

    #[test]
    fn text_follows_the_target_type() {
        assert_eq!(text_to_bson("42", Target::Int32).unwrap(), Bson::Int32(42));
        assert_eq!(text_to_bson("42", Target::Int64).unwrap(), Bson::Int64(42));
        assert_eq!(
            text_to_bson("99999999999", Target::Int32).unwrap(),
            Bson::Int64(99_999_999_999)
        );
        assert_eq!(
            text_to_bson("abc", Target::Int32).unwrap(),
            Bson::String("abc".into())
        );
        assert_eq!(
            text_to_bson("yes", Target::Bool).unwrap(),
            Bson::Boolean(true)
        );
        assert!(matches!(
            text_to_bson("2024-05-01", Target::DateTime).unwrap(),
            Bson::DateTime(_)
        ));
        assert!(text_to_bson("not json", Target::Container).is_err());
    }
}
