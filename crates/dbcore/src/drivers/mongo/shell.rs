//! A parser for the mongosh subset TableSail runs: one statement at a time
//! (the splitter is someone else's), turned into a typed `Statement`.
//!
//! Arguments are parsed by rewriting mongosh constructors (`ObjectId("..")`,
//! `ISODate("..")`, `/re/i`, ...) to Extended JSON, reading the result with
//! json5 (unquoted keys, single quotes, trailing commas) and converting the
//! JSON to BSON, so every type survives.

use base64::Engine as _;
use mongodb::bson::{Bson, Document, doc, oid::ObjectId};
use mongodb::options::UpdateModifications;
use serde_json::{Value as Json, json};

use super::convert::{json_to_bson, parse_datetime, type_name};
use crate::error::{Error, Result};

/// An update document or aggregation pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateSpec {
    Document(Document),
    Pipeline(Vec<Document>),
}

impl From<UpdateSpec> for UpdateModifications {
    fn from(spec: UpdateSpec) -> Self {
        match spec {
            UpdateSpec::Document(d) => UpdateModifications::Document(d),
            UpdateSpec::Pipeline(p) => UpdateModifications::Pipeline(p),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    /// Set by `db.getSiblingDB("name")`.
    pub database: Option<String>,
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindOp {
    pub collection: String,
    pub filter: Document,
    pub projection: Option<Document>,
    pub sort: Option<Document>,
    pub skip: u64,
    /// None or Some(0) mean "no limit".
    pub limit: Option<i64>,
    pub one: bool,
    pub hint: Option<Bson>,
    pub max_time_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    ShowDatabases,
    ShowCollections,
    RunCommand {
        command: Document,
        admin: bool,
    },
    GetCollectionNames,
    CreateCollection {
        name: String,
        options: Document,
    },
    DropDatabase,
    Find(Box<FindOp>),
    Aggregate {
        collection: String,
        pipeline: Vec<Document>,
        allow_disk_use: bool,
    },
    CountDocuments {
        collection: String,
        filter: Document,
        limit: Option<u64>,
        skip: Option<u64>,
    },
    EstimatedCount {
        collection: String,
    },
    Distinct {
        collection: String,
        field: String,
        filter: Document,
    },
    InsertOne {
        collection: String,
        document: Document,
    },
    InsertMany {
        collection: String,
        documents: Vec<Document>,
        ordered: bool,
    },
    Update {
        collection: String,
        filter: Document,
        update: UpdateSpec,
        many: bool,
        upsert: bool,
        array_filters: Option<Vec<Document>>,
    },
    Replace {
        collection: String,
        filter: Document,
        replacement: Document,
        upsert: bool,
    },
    Delete {
        collection: String,
        filter: Document,
        many: bool,
    },
    CreateIndex {
        collection: String,
        keys: Document,
        options: Document,
    },
    DropIndex {
        collection: String,
        index: Bson,
    },
    GetIndexes {
        collection: String,
    },
    Drop {
        collection: String,
    },
}

impl Op {
    /// True when the statement changes data or schema (the cached structure may be stale).
    pub fn modifies_schema(&self) -> bool {
        matches!(
            self,
            Op::CreateCollection { .. }
                | Op::DropDatabase
                | Op::CreateIndex { .. }
                | Op::DropIndex { .. }
                | Op::Drop { .. }
                | Op::RunCommand { .. }
        )
    }
}

fn bad(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

// ---------------------------------------------------------------------------
// Lexical helpers
// ---------------------------------------------------------------------------

/// Marks bytes of string and regex literals (all `LIT`) and comments (spaces),
/// keeping offsets identical, so bracket matching and comma splitting can run
/// on the mask while slices are taken from the original text.
const LIT: u8 = 1;

fn regex_allowed(prev: u8) -> bool {
    prev == 0
        || matches!(
            prev,
            b'(' | b',' | b':' | b'[' | b'{' | b'=' | b'!' | b'&' | b'|' | b'?'
        )
}

fn mask(src: &str) -> Result<Vec<u8>> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let mut i = 0;
    let mut prev = 0u8;
    while i < n {
        let c = b[i];
        match c {
            b'"' | b'\'' | b'`' => {
                let mut j = i + 1;
                loop {
                    if j >= n {
                        return Err(bad("Unterminated string literal"));
                    }
                    if b[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if b[j] == c {
                        break;
                    }
                    j += 1;
                }
                out[i..=j].fill(LIT);
                i = j + 1;
                prev = b'a';
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(n, |p| i + p);
                out[i..end].fill(b' ');
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map_or(n, |p| i + 2 + p + 2);
                out[i..end].fill(b' ');
                i = end;
            }
            b'/' if regex_allowed(prev) => {
                let mut j = i + 1;
                let mut in_class = false;
                loop {
                    if j >= n {
                        return Err(bad("Unterminated regular expression"));
                    }
                    match b[j] {
                        b'\\' => {
                            j += 2;
                            continue;
                        }
                        b'[' => in_class = true,
                        b']' => in_class = false,
                        b'/' if !in_class => break,
                        b'\n' => return Err(bad("Unterminated regular expression")),
                        _ => {}
                    }
                    j += 1;
                }
                j += 1;
                while j < n && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                out[i..j].fill(LIT);
                i = j;
                prev = b'a';
            }
            c if c.is_ascii_whitespace() => i += 1,
            c => {
                prev = c;
                i += 1;
            }
        }
    }
    Ok(out)
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Index of the bracket closing the one at `open`.
fn find_close(mask: &[u8], open: usize) -> Result<usize> {
    let mut depth = 0usize;
    for (i, &c) in mask.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err(bad("Unbalanced brackets"))
}

/// Splits `src[start..end]` at top-level commas. A single trailing comma and
/// an empty list are tolerated.
fn split_args(src: &str, mask: &[u8], start: usize, end: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut from = start;
    for i in start..end {
        match mask[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(src[from..i].trim().to_owned());
                from = i + 1;
            }
            _ => {}
        }
    }
    let last = src[from..end].trim();
    if !last.is_empty() {
        parts.push(last.to_owned());
    }
    parts
}

// ---------------------------------------------------------------------------
// Values: mongosh -> Extended JSON -> BSON
// ---------------------------------------------------------------------------

/// Parses one mongosh/JSON value (document, array, scalar).
pub fn parse_value(text: &str) -> Result<Bson> {
    if text.trim().is_empty() {
        return Err(bad("Missing value"));
    }
    let rewritten = rewrite(text)?;
    let json: Json = json5::from_str(&rewritten).map_err(|e| bad(format!("Invalid value: {e}")))?;
    json_to_bson(json)
}

pub fn parse_document(text: &str) -> Result<Document> {
    match parse_value(text)? {
        Bson::Document(d) => Ok(d),
        other => Err(bad(format!(
            "Expected a document, found {}",
            type_name(&other)
        ))),
    }
}

fn parse_array(text: &str) -> Result<Vec<Bson>> {
    match parse_value(text)? {
        Bson::Array(a) => Ok(a),
        other => Err(bad(format!(
            "Expected an array, found {}",
            type_name(&other)
        ))),
    }
}

fn json_string(s: &str) -> String {
    Json::String(s.to_owned()).to_string()
}

/// Rewrites mongosh constructors and regex literals to Extended JSON text.
fn rewrite(src: &str) -> Result<String> {
    let m = mask(src)?;
    let n = src.len();
    let mut out = String::with_capacity(n + 16);
    let mut i = 0;
    while i < n {
        let c = m[i];
        if c == LIT {
            let mut j = i;
            while j < n && m[j] == LIT {
                j += 1;
            }
            let lit = &src[i..j];
            match lit.as_bytes()[0] {
                b'/' => out.push_str(&regex_json(lit)?.to_string()),
                b'`' => out.push_str(&json_string(&lit[1..lit.len() - 1])),
                _ => out.push_str(lit),
            }
            i = j;
        } else if is_ident_start(c) {
            let mut j = i;
            while j < n && is_ident(m[j]) {
                j += 1;
            }
            let name = &src[i..j];
            i = j;
            match name {
                "new" => {}
                "undefined" => out.push_str("null"),
                "NaN" => out.push_str(r#"{"$numberDouble":"NaN"}"#),
                "Infinity" => {
                    if out.trim_end().ends_with('-') {
                        let kept = out.trim_end().len() - 1;
                        out.truncate(kept);
                        out.push_str(r#"{"$numberDouble":"-Infinity"}"#);
                    } else {
                        out.push_str(r#"{"$numberDouble":"Infinity"}"#);
                    }
                }
                "MinKey" | "MaxKey" => {
                    out.push_str(if name == "MinKey" {
                        r#"{"$minKey":1}"#
                    } else {
                        r#"{"$maxKey":1}"#
                    });
                    // Optional empty parentheses: MinKey().
                    let mut k = i;
                    while k < n && src.as_bytes()[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if m.get(k) == Some(&b'(') {
                        i = find_close(&m, k)? + 1;
                    }
                }
                _ if is_constructor(name) => {
                    let mut k = i;
                    while k < n && src.as_bytes()[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if m.get(k) == Some(&b'(') {
                        let close = find_close(&m, k)?;
                        let args = split_args(src, &m, k + 1, close);
                        out.push_str(&constructor(name, &args)?.to_string());
                        i = close + 1;
                    } else {
                        out.push_str(name);
                    }
                }
                _ => out.push_str(name),
            }
        } else {
            // Copy one char (keeps multi-byte characters intact); comments were masked to spaces.
            let ch = src[i..].chars().next().unwrap_or(' ');
            out.push(if c == b' ' && !ch.is_ascii_whitespace() {
                ' '
            } else {
                ch
            });
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

fn is_constructor(name: &str) -> bool {
    matches!(
        name,
        "ObjectId"
            | "ObjectID"
            | "ISODate"
            | "Date"
            | "NumberLong"
            | "Long"
            | "NumberInt"
            | "Int32"
            | "NumberDecimal"
            | "Decimal128"
            | "UUID"
            | "BinData"
            | "HexData"
            | "Timestamp"
            | "RegExp"
    )
}

fn regex_json(literal: &str) -> Result<Json> {
    let end = literal
        .rfind('/')
        .filter(|&p| p > 0)
        .ok_or_else(|| bad("Invalid regular expression"))?;
    let (raw, options) = (&literal[1..end], &literal[end + 1..]);
    // `\/` only exists to keep the literal valid; the pattern wants a plain slash.
    let mut pattern = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('/') => pattern.push('/'),
                Some(next) => {
                    pattern.push('\\');
                    pattern.push(next);
                }
                None => pattern.push('\\'),
            }
        } else {
            pattern.push(c);
        }
    }
    Ok(json!({"$regularExpression": {"pattern": pattern, "options": options}}))
}

fn int_arg(b: &Bson, what: &str) -> Result<i64> {
    match b {
        Bson::Int32(i) => Ok(i64::from(*i)),
        Bson::Int64(i) => Ok(*i),
        Bson::Double(f) if f.fract() == 0.0 && f.abs() < 9.2e18 => Ok(*f as i64),
        Bson::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|_| bad(format!("{what}: {s:?} is not an integer"))),
        other => Err(bad(format!(
            "{what}: expected an integer, found {}",
            type_name(other)
        ))),
    }
}

fn constructor(name: &str, args: &[String]) -> Result<Json> {
    let values = args
        .iter()
        .map(|a| parse_value(a))
        .collect::<Result<Vec<_>>>()?;
    let first = values.first();
    match name {
        "ObjectId" | "ObjectID" => {
            let hex = match first {
                None => ObjectId::new().to_hex(),
                Some(Bson::String(s)) => ObjectId::parse_str(s.trim())
                    .map_err(|_| bad(format!("ObjectId: {s:?} is not 24 hex characters")))?
                    .to_hex(),
                Some(Bson::ObjectId(o)) => o.to_hex(),
                Some(other) => {
                    return Err(bad(format!(
                        "ObjectId: expected a string, found {}",
                        type_name(other)
                    )));
                }
            };
            Ok(json!({"$oid": hex}))
        }
        "ISODate" | "Date" => {
            let ms = match first {
                None => mongodb::bson::DateTime::now().timestamp_millis(),
                Some(Bson::String(s)) => {
                    parse_datetime(s).ok_or_else(|| bad(format!("Invalid date {s:?}")))?
                }
                Some(Bson::DateTime(d)) => d.timestamp_millis(),
                Some(other) => int_arg(other, "Date")?,
            };
            Ok(json!({"$date": {"$numberLong": ms.to_string()}}))
        }
        "NumberLong" | "Long" => {
            let v = first.ok_or_else(|| bad("NumberLong needs a value"))?;
            Ok(json!({"$numberLong": int_arg(v, "NumberLong")?.to_string()}))
        }
        "NumberInt" | "Int32" => {
            let v = first.ok_or_else(|| bad("NumberInt needs a value"))?;
            let i = int_arg(v, "NumberInt")?;
            i32::try_from(i).map_err(|_| bad(format!("NumberInt: {i} does not fit 32 bits")))?;
            Ok(json!({"$numberInt": i.to_string()}))
        }
        "NumberDecimal" | "Decimal128" => {
            let text = match first {
                Some(Bson::String(s)) => s.trim().to_owned(),
                Some(Bson::Int32(i)) => i.to_string(),
                Some(Bson::Int64(i)) => i.to_string(),
                Some(Bson::Double(f)) => f.to_string(),
                Some(Bson::Decimal128(d)) => d.to_string(),
                _ => return Err(bad("NumberDecimal needs a numeric string")),
            };
            text.parse::<mongodb::bson::Decimal128>()
                .map_err(|e| bad(format!("NumberDecimal: invalid decimal {text:?}: {e}")))?;
            Ok(json!({"$numberDecimal": text}))
        }
        "UUID" => {
            let uuid = match first {
                None => uuid::Uuid::new_v4(),
                Some(Bson::String(s)) => uuid::Uuid::parse_str(s.trim())
                    .map_err(|e| bad(format!("UUID: invalid value {s:?}: {e}")))?,
                Some(other) => {
                    return Err(bad(format!(
                        "UUID: expected a string, found {}",
                        type_name(other)
                    )));
                }
            };
            Ok(binary_json(uuid.as_bytes(), 4))
        }
        "BinData" => {
            let subtype = int_arg(
                first.ok_or_else(|| bad("BinData(subtype, base64)"))?,
                "BinData",
            )?;
            let data = match values.get(1) {
                Some(Bson::String(s)) => base64::engine::general_purpose::STANDARD
                    .decode(s.trim())
                    .map_err(|e| bad(format!("BinData: invalid base64: {e}")))?,
                _ => return Err(bad("BinData(subtype, base64)")),
            };
            Ok(binary_json(
                &data,
                u8::try_from(subtype).map_err(|_| bad("BinData: invalid subtype"))?,
            ))
        }
        "HexData" => {
            let subtype = int_arg(
                first.ok_or_else(|| bad("HexData(subtype, hex)"))?,
                "HexData",
            )?;
            let data = match values.get(1) {
                Some(Bson::String(s)) => {
                    hex::decode(s.trim()).map_err(|e| bad(format!("HexData: invalid hex: {e}")))?
                }
                _ => return Err(bad("HexData(subtype, hex)")),
            };
            Ok(binary_json(
                &data,
                u8::try_from(subtype).map_err(|_| bad("HexData: invalid subtype"))?,
            ))
        }
        "Timestamp" => {
            let (t, i) = match (values.first(), values.get(1)) {
                (Some(t), Some(i)) => (int_arg(t, "Timestamp")?, int_arg(i, "Timestamp")?),
                _ => return Err(bad("Timestamp(seconds, increment)")),
            };
            Ok(json!({"$timestamp": {"t": t, "i": i}}))
        }
        "RegExp" => {
            let pattern = match first {
                Some(Bson::String(s)) => s.clone(),
                Some(Bson::RegularExpression(r)) => r.pattern.clone(),
                _ => return Err(bad("RegExp(pattern, flags)")),
            };
            let options = match values.get(1) {
                Some(Bson::String(s)) => s.clone(),
                _ => String::new(),
            };
            Ok(json!({"$regularExpression": {"pattern": pattern, "options": options}}))
        }
        _ => Err(bad(format!("Unknown constructor {name}"))),
    }
}

fn binary_json(bytes: &[u8], subtype: u8) -> Json {
    json!({"$binary": {
        "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
        "subType": format!("{subtype:02x}"),
    }})
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

struct Call {
    name: String,
    args: Vec<String>,
}

struct Parser<'a> {
    src: &'a str,
    mask: Vec<u8>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.mask.len() && self.mask[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.mask.get(self.pos).copied()
    }

    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.mask.len()
    }

    fn ident(&mut self) -> Option<&'a str> {
        self.skip_ws();
        let start = self.pos;
        if !self.peek().is_some_and(is_ident_start) {
            return None;
        }
        while self.peek().is_some_and(is_ident) {
            self.pos += 1;
        }
        Some(&self.src[start..self.pos])
    }

    /// Parses `( args )` at the current position.
    fn args(&mut self) -> Result<Vec<String>> {
        self.skip_ws();
        if self.peek() != Some(b'(') {
            return Err(bad("Expected '('"));
        }
        let close = find_close(&self.mask, self.pos)?;
        let args = split_args(self.src, &self.mask, self.pos + 1, close);
        self.pos = close + 1;
        Ok(args)
    }

    fn string_arg(&self, args: &[String], what: &str) -> Result<String> {
        match args.first().map(|a| parse_value(a)).transpose()? {
            Some(Bson::String(s)) => Ok(s),
            _ => Err(bad(format!("{what} expects a string"))),
        }
    }
}

fn opt_doc(args: &[String], i: usize) -> Result<Option<Document>> {
    match args.get(i) {
        Some(a) if !a.is_empty() => Ok(Some(parse_document(a)?)),
        _ => Ok(None),
    }
}

fn doc_or_empty(args: &[String], i: usize) -> Result<Document> {
    Ok(opt_doc(args, i)?.unwrap_or_default())
}

fn required_doc(args: &[String], i: usize, method: &str) -> Result<Document> {
    opt_doc(args, i)?.ok_or_else(|| bad(format!("{method}() is missing a document argument")))
}

fn flag(options: &Option<Document>, key: &str) -> bool {
    options
        .as_ref()
        .and_then(|d| d.get(key))
        .is_some_and(|b| matches!(b, Bson::Boolean(true)))
}

fn number(text: &str, what: &str) -> Result<i64> {
    int_arg(&parse_value(text)?, what)
}

fn strip_terminator(input: &str) -> &str {
    let mut s = input.trim();
    while let Some(rest) = s.strip_suffix(';') {
        s = rest.trim_end();
    }
    s
}

/// Parses one statement.
pub fn parse(input: &str) -> Result<Statement> {
    let src = strip_terminator(input);
    let mask = mask(src)?;
    let mut p = Parser { src, mask, pos: 0 };
    if p.at_end() {
        return Err(bad("Empty statement"));
    }

    if p.peek() == Some(b'{') {
        let command = parse_document(&src[p.pos..])?;
        return Ok(Statement {
            database: None,
            op: Op::RunCommand {
                command,
                admin: false,
            },
        });
    }

    let first = p
        .ident()
        .ok_or_else(|| bad("Expected a statement such as db.collection.find({})"))?;
    match first {
        "show" => {
            let what = p.ident().unwrap_or_default().to_ascii_lowercase();
            let op = match what.as_str() {
                "dbs" | "databases" => Op::ShowDatabases,
                "collections" | "tables" => Op::ShowCollections,
                other => return Err(Error::Unsupported(format!("show {other} is not supported"))),
            };
            return Ok(Statement { database: None, op });
        }
        "use" => {
            return Err(bad(
                "`use` is not needed: choose the database in the sidebar or with db.getSiblingDB(\"name\")",
            ));
        }
        "db" => {}
        other => {
            return Err(bad(format!(
                "Unsupported statement starting with `{other}`"
            )));
        }
    }

    let mut database = None;
    let mut collection: Option<String> = None;
    let mut parts: Vec<String> = Vec::new();
    let root = loop {
        p.skip_ws();
        match p.peek() {
            Some(b'.') => {
                p.pos += 1;
                let name = p.ident().ok_or_else(|| bad("Expected a name after '.'"))?;
                p.skip_ws();
                if p.peek() == Some(b'(') {
                    let args = p.args()?;
                    match name {
                        "getCollection" => {
                            collection = Some(p.string_arg(&args, "getCollection")?);
                            parts.clear();
                        }
                        "getSiblingDB" => database = Some(p.string_arg(&args, "getSiblingDB")?),
                        _ => {
                            break Call {
                                name: name.to_owned(),
                                args,
                            };
                        }
                    }
                } else {
                    parts.push(name.to_owned());
                }
            }
            Some(b'[') => {
                let close = find_close(&p.mask, p.pos)?;
                let inner = src[p.pos + 1..close].trim();
                match parse_value(inner)? {
                    Bson::String(s) => {
                        if collection.is_none() && parts.is_empty() {
                            collection = Some(s);
                        } else {
                            parts.push(s);
                        }
                    }
                    _ => return Err(bad("Expected a collection name in brackets")),
                }
                p.pos = close + 1;
            }
            _ => return Err(bad("Expected a method call such as .find({})")),
        }
    };

    let mut modifiers = Vec::new();
    while !p.at_end() {
        if p.peek() != Some(b'.') {
            return Err(bad("Unexpected text after the statement"));
        }
        p.pos += 1;
        let name = p
            .ident()
            .ok_or_else(|| bad("Expected a method name after '.'"))?;
        let args = p.args()?;
        modifiers.push(Call {
            name: name.to_owned(),
            args,
        });
    }

    let coll = match (&collection, parts.is_empty()) {
        (Some(c), true) => Some(c.clone()),
        (Some(c), false) => Some(format!("{c}.{}", parts.join("."))),
        (None, false) => Some(parts.join(".")),
        (None, true) => None,
    };
    let op = match coll {
        None => db_op(&root, &modifiers)?,
        Some(collection) => collection_op(collection, &root, modifiers)?,
    };
    Ok(Statement { database, op })
}

fn db_op(root: &Call, modifiers: &[Call]) -> Result<Op> {
    if let Some(m) = modifiers.first() {
        return Err(bad(format!(
            "Unexpected .{}() after {}()",
            m.name, root.name
        )));
    }
    let args = &root.args;
    Ok(match root.name.as_str() {
        "runCommand" | "adminCommand" => {
            let command = match args.first().map(|a| parse_value(a)).transpose()? {
                Some(Bson::Document(d)) => d,
                Some(Bson::String(s)) => doc! { s: 1 },
                _ => return Err(bad(format!("{}() expects a command document", root.name))),
            };
            Op::RunCommand {
                command,
                admin: root.name == "adminCommand",
            }
        }
        "getCollectionNames" => Op::GetCollectionNames,
        "createCollection" => {
            let name = match args.first().map(|a| parse_value(a)).transpose()? {
                Some(Bson::String(s)) => s,
                _ => return Err(bad("createCollection() expects a collection name")),
            };
            Op::CreateCollection {
                name,
                options: doc_or_empty(args, 1)?,
            }
        }
        "dropDatabase" => Op::DropDatabase,
        other => {
            return Err(Error::Unsupported(format!(
                "db.{other}() is not supported; use db.runCommand({{...}})"
            )));
        }
    })
}

fn collection_op(collection: String, root: &Call, modifiers: Vec<Call>) -> Result<Op> {
    let args = &root.args;
    let method = root.name.as_str();
    let is_find = matches!(method, "find" | "findOne");
    if !is_find {
        for m in &modifiers {
            if !matches!(m.name.as_str(), "toArray" | "pretty") {
                return Err(bad(format!("Unexpected .{}() after {method}()", m.name)));
            }
        }
    }
    Ok(match method {
        "find" | "findOne" => {
            let mut f = FindOp {
                collection: collection.clone(),
                filter: doc_or_empty(args, 0)?,
                projection: opt_doc(args, 1)?,
                sort: None,
                skip: 0,
                limit: None,
                one: method == "findOne",
                hint: None,
                max_time_ms: None,
            };
            let mut count = false;
            for m in modifiers {
                let a = &m.args;
                match m.name.as_str() {
                    "sort" => f.sort = Some(required_doc(a, 0, "sort")?),
                    "project" | "projection" => f.projection = Some(required_doc(a, 0, "project")?),
                    "skip" => {
                        f.skip = number(a.first().map_or("", String::as_str), "skip")?.max(0) as u64
                    }
                    "limit" => {
                        f.limit = Some(number(a.first().map_or("", String::as_str), "limit")?.abs())
                    }
                    "maxTimeMS" => {
                        f.max_time_ms = Some(
                            number(a.first().map_or("", String::as_str), "maxTimeMS")?.max(0)
                                as u64,
                        )
                    }
                    "hint" => f.hint = Some(parse_value(a.first().map_or("", String::as_str))?),
                    "toArray" | "pretty" | "batchSize" | "noCursorTimeout" | "allowDiskUse" => {}
                    "count" | "itcount" | "size" => count = true,
                    other => {
                        return Err(Error::Unsupported(format!(
                            "Cursor method .{other}() is not supported"
                        )));
                    }
                }
            }
            if count {
                Op::CountDocuments {
                    collection,
                    filter: f.filter,
                    limit: f.limit.filter(|l| *l > 0).map(|l| l as u64),
                    skip: Some(f.skip).filter(|s| *s > 0),
                }
            } else {
                Op::Find(Box::new(f))
            }
        }
        "aggregate" => {
            let pipeline = match args.first() {
                Some(a) => parse_array(a)?
                    .into_iter()
                    .map(|stage| match stage {
                        Bson::Document(d) => Ok(d),
                        other => Err(bad(format!(
                            "Pipeline stages must be documents, found {}",
                            type_name(&other)
                        ))),
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => return Err(bad("aggregate() expects a pipeline array")),
            };
            Op::Aggregate {
                collection,
                pipeline,
                allow_disk_use: flag(&opt_doc(args, 1)?, "allowDiskUse"),
            }
        }
        "countDocuments" | "count" => {
            let options = opt_doc(args, 1)?;
            let num = |key: &str| {
                options
                    .as_ref()
                    .and_then(|d| d.get(key))
                    .and_then(|b| int_arg(b, key).ok())
                    .filter(|n| *n > 0)
                    .map(|n| n as u64)
            };
            Op::CountDocuments {
                collection,
                filter: doc_or_empty(args, 0)?,
                limit: num("limit"),
                skip: num("skip"),
            }
        }
        "estimatedDocumentCount" => Op::EstimatedCount { collection },
        "distinct" => {
            let field = match args.first().map(|a| parse_value(a)).transpose()? {
                Some(Bson::String(s)) => s,
                _ => return Err(bad("distinct() expects a field name")),
            };
            Op::Distinct {
                collection,
                field,
                filter: doc_or_empty(args, 1)?,
            }
        }
        "insertOne" => Op::InsertOne {
            collection,
            document: required_doc(args, 0, "insertOne")?,
        },
        "insertMany" | "insert" => {
            let first = args
                .first()
                .ok_or_else(|| bad(format!("{method}() expects documents")))?;
            match parse_value(first)? {
                Bson::Array(items) => Op::InsertMany {
                    collection,
                    documents: items
                        .into_iter()
                        .map(|b| match b {
                            Bson::Document(d) => Ok(d),
                            other => Err(bad(format!(
                                "insertMany() takes documents, found {}",
                                type_name(&other)
                            ))),
                        })
                        .collect::<Result<_>>()?,
                    ordered: !matches!(
                        opt_doc(args, 1)?.and_then(|d| d.get("ordered").cloned()),
                        Some(Bson::Boolean(false))
                    ),
                },
                Bson::Document(document) if method == "insert" => Op::InsertOne {
                    collection,
                    document,
                },
                other => {
                    return Err(bad(format!(
                        "{method}() expects an array of documents, found {}",
                        type_name(&other)
                    )));
                }
            }
        }
        "updateOne" | "updateMany" => {
            let options = opt_doc(args, 2)?;
            let update = match args.get(1).map(|a| parse_value(a)).transpose()? {
                Some(Bson::Document(d)) => UpdateSpec::Document(d),
                Some(Bson::Array(stages)) => UpdateSpec::Pipeline(
                    stages
                        .into_iter()
                        .map(|s| match s {
                            Bson::Document(d) => Ok(d),
                            other => Err(bad(format!(
                                "Pipeline stages must be documents, found {}",
                                type_name(&other)
                            ))),
                        })
                        .collect::<Result<_>>()?,
                ),
                _ => return Err(bad(format!("{method}() expects an update document"))),
            };
            let array_filters = match options.as_ref().and_then(|d| d.get("arrayFilters")) {
                Some(Bson::Array(items)) => Some(
                    items
                        .iter()
                        .filter_map(|b| b.as_document().cloned())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            };
            Op::Update {
                collection,
                filter: required_doc(args, 0, method)?,
                update,
                many: method == "updateMany",
                upsert: flag(&options, "upsert"),
                array_filters,
            }
        }
        "replaceOne" => Op::Replace {
            collection,
            filter: required_doc(args, 0, method)?,
            replacement: required_doc(args, 1, method)?,
            upsert: flag(&opt_doc(args, 2)?, "upsert"),
        },
        "deleteOne" | "deleteMany" => Op::Delete {
            collection,
            filter: required_doc(args, 0, method)?,
            many: method == "deleteMany",
        },
        "createIndex" | "ensureIndex" => Op::CreateIndex {
            collection,
            keys: required_doc(args, 0, method)?,
            options: doc_or_empty(args, 1)?,
        },
        "dropIndex" => Op::DropIndex {
            collection,
            index: match args.first() {
                Some(a) => parse_value(a)?,
                None => return Err(bad("dropIndex() expects an index name or key document")),
            },
        },
        "getIndexes" => Op::GetIndexes { collection },
        "drop" => Op::Drop { collection },
        other => {
            return Err(Error::Unsupported(format!(
                "{other}() is not supported on collections"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{DateTime, Regex, Timestamp};

    fn value(text: &str) -> Bson {
        parse_value(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn relaxed_syntax() {
        let b = value("{ name: 'x', \"b\": [1, 2,], c: { $gt: -5 }, }");
        assert_eq!(
            b,
            Bson::Document(doc! { "name": "x", "b": [1, 2], "c": { "$gt": -5 } })
        );
    }

    #[test]
    fn numbers_keep_their_width() {
        let b = value("{ a: 1, b: 5000000000, c: 1.5, d: 2.0, e: -3 }");
        let d = b.as_document().unwrap();
        assert_eq!(d.get("a"), Some(&Bson::Int32(1)));
        assert_eq!(d.get("b"), Some(&Bson::Int64(5_000_000_000)));
        assert_eq!(d.get("c"), Some(&Bson::Double(1.5)));
        assert_eq!(d.get("d"), Some(&Bson::Double(2.0)));
        assert_eq!(d.get("e"), Some(&Bson::Int32(-3)));
    }

    #[test]
    fn constructors() {
        let b = value(
            r#"{
            _id: ObjectId("65f000000000000000000001"),
            at: ISODate("2024-05-01T12:34:56.789Z"),
            at2: new Date("2024-05-01"),
            n: NumberLong("9007199254740993"),
            i: NumberInt(5),
            d: NumberDecimal("1.50"),
            u: UUID("01020304-0506-0708-090a-0b0c0d0e0f10"),
            ts: Timestamp(10, 2),
            lo: MinKey(),
            hi: MaxKey,
            inf: -Infinity,
            nada: undefined,
        }"#,
        );
        let d = b.as_document().unwrap();
        assert!(matches!(d.get("_id"), Some(Bson::ObjectId(_))));
        assert_eq!(
            d.get("at"),
            Some(&Bson::DateTime(DateTime::from_millis(1_714_566_896_789)))
        );
        assert_eq!(
            d.get("at2"),
            Some(&Bson::DateTime(DateTime::from_millis(1_714_521_600_000)))
        );
        assert_eq!(d.get("n"), Some(&Bson::Int64(9_007_199_254_740_993)));
        assert_eq!(d.get("i"), Some(&Bson::Int32(5)));
        assert_eq!(d.get("d"), Some(&Bson::Decimal128("1.50".parse().unwrap())));
        match d.get("u") {
            Some(Bson::Binary(b)) => {
                assert_eq!(b.subtype, mongodb::bson::spec::BinarySubtype::Uuid);
                assert_eq!(b.bytes.len(), 16);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            d.get("ts"),
            Some(&Bson::Timestamp(Timestamp {
                time: 10,
                increment: 2
            }))
        );
        assert_eq!(d.get("lo"), Some(&Bson::MinKey));
        assert_eq!(d.get("hi"), Some(&Bson::MaxKey));
        assert_eq!(d.get("inf"), Some(&Bson::Double(f64::NEG_INFINITY)));
        assert_eq!(d.get("nada"), Some(&Bson::Null));
    }

    #[test]
    fn regex_literals_and_strings_with_brackets() {
        let b = value(r#"{ a: /^ab[/]c,d$/i, b: "x/y (z", c: 'it\'s' }"#);
        let d = b.as_document().unwrap();
        assert_eq!(
            d.get("a"),
            Some(&Bson::RegularExpression(Regex {
                pattern: "^ab[/]c,d$".into(),
                options: "i".into()
            }))
        );
        assert_eq!(d.get("b"), Some(&Bson::String("x/y (z".into())));
        assert_eq!(d.get("c"), Some(&Bson::String("it's".into())));
    }

    #[test]
    fn non_ascii_text_survives() {
        let b = value("{ a: 'é', /* ü */ b: \"日本\", ñ: 1 } // ñ");
        assert_eq!(b, Bson::Document(doc! {"a": "é", "b": "日本", "ñ": 1}));
        let s = parse("db.getCollection('café').find({ név: 'x' })").unwrap();
        let Op::Find(f) = s.op else { panic!() };
        assert_eq!(f.collection, "café");
    }

    #[test]
    fn comments_are_ignored() {
        let b = value("{ a: 1, // one\n b: /* two */ 2 }");
        assert_eq!(b, Bson::Document(doc! {"a": 1, "b": 2}));
    }

    #[test]
    fn find_with_chain() {
        let s = parse(
            r#"db.users.find({ age: { $gt: 18 } }, { name: 1 }).sort({ age: -1 }).skip(10).limit(5);"#,
        )
        .unwrap();
        assert_eq!(s.database, None);
        let Op::Find(f) = s.op else { panic!() };
        assert_eq!(f.collection, "users");
        assert_eq!(f.filter, doc! { "age": { "$gt": 18 } });
        assert_eq!(f.projection, Some(doc! { "name": 1 }));
        assert_eq!(f.sort, Some(doc! { "age": -1 }));
        assert_eq!((f.skip, f.limit, f.one), (10, Some(5), false));
    }

    #[test]
    fn get_collection_and_dotted_names() {
        let s = parse(r#"db.getCollection("my-coll").findOne({ _id: 1 })"#).unwrap();
        let Op::Find(f) = s.op else { panic!() };
        assert_eq!((f.collection.as_str(), f.one), ("my-coll", true));

        let s = parse("db.system.users.find()").unwrap();
        let Op::Find(f) = s.op else { panic!() };
        assert_eq!(f.collection, "system.users");

        let s = parse(r#"db.getSiblingDB("other").logs.find({})"#).unwrap();
        assert_eq!(s.database.as_deref(), Some("other"));
    }

    #[test]
    fn aggregate_and_counts() {
        let s = parse(
            r#"db.orders.aggregate([{ $match: { status: "A" } }, { $group: { _id: "$cust", n: { $sum: 1 } } }], { allowDiskUse: true })"#,
        )
        .unwrap();
        let Op::Aggregate {
            pipeline,
            allow_disk_use,
            ..
        } = s.op
        else {
            panic!()
        };
        assert_eq!(pipeline.len(), 2);
        assert!(allow_disk_use);

        assert!(matches!(
            parse("db.c.countDocuments({a: 1})").unwrap().op,
            Op::CountDocuments { .. }
        ));
        assert!(matches!(
            parse("db.c.estimatedDocumentCount()").unwrap().op,
            Op::EstimatedCount { .. }
        ));
        assert!(matches!(
            parse("db.c.find({a: 1}).count()").unwrap().op,
            Op::CountDocuments { .. }
        ));
        let Op::Distinct { field, filter, .. } = parse(r#"db.c.distinct("a", {b: 1})"#).unwrap().op
        else {
            panic!()
        };
        assert_eq!(field, "a");
        assert_eq!(filter, doc! {"b": 1});
    }

    #[test]
    fn writes() {
        assert!(matches!(
            parse("db.c.insertOne({ a: 1 })").unwrap().op,
            Op::InsertOne { .. }
        ));
        let Op::InsertMany {
            documents, ordered, ..
        } = parse("db.c.insertMany([{a: 1}, {a: 2}], { ordered: false })")
            .unwrap()
            .op
        else {
            panic!()
        };
        assert_eq!((documents.len(), ordered), (2, false));

        let Op::Update {
            many,
            upsert,
            update,
            ..
        } = parse("db.c.updateMany({a: 1}, {$set: {b: 2}}, {upsert: true})")
            .unwrap()
            .op
        else {
            panic!()
        };
        assert!(many && upsert);
        assert_eq!(update, UpdateSpec::Document(doc! {"$set": {"b": 2}}));

        let Op::Update { update, .. } = parse("db.c.updateOne({a: 1}, [{$set: {b: 2}}])")
            .unwrap()
            .op
        else {
            panic!()
        };
        assert!(matches!(update, UpdateSpec::Pipeline(_)));

        assert!(matches!(
            parse("db.c.replaceOne({a: 1}, {a: 2})").unwrap().op,
            Op::Replace { .. }
        ));
        assert!(matches!(
            parse("db.c.deleteMany({})").unwrap().op,
            Op::Delete { many: true, .. }
        ));
        assert!(matches!(
            parse("db.c.createIndex({a: 1}, {unique: true})")
                .unwrap()
                .op,
            Op::CreateIndex { .. }
        ));
        assert!(matches!(
            parse(r#"db.c.dropIndex("a_1")"#).unwrap().op,
            Op::DropIndex { .. }
        ));
        assert!(matches!(parse("db.c.drop()").unwrap().op, Op::Drop { .. }));
    }

    #[test]
    fn commands_and_show() {
        assert_eq!(parse("show dbs").unwrap().op, Op::ShowDatabases);
        assert_eq!(parse("show collections;").unwrap().op, Op::ShowCollections);
        assert_eq!(
            parse("db.runCommand({ ping: 1 })").unwrap().op,
            Op::RunCommand {
                command: doc! {"ping": 1},
                admin: false
            }
        );
        assert_eq!(
            parse("db.adminCommand({ listDatabases: 1 })").unwrap().op,
            Op::RunCommand {
                command: doc! {"listDatabases": 1},
                admin: true
            }
        );
        assert_eq!(
            parse(r#"{ "buildInfo": 1 }"#).unwrap().op,
            Op::RunCommand {
                command: doc! {"buildInfo": 1},
                admin: false
            }
        );
        assert_eq!(
            parse("db.getCollectionNames()").unwrap().op,
            Op::GetCollectionNames
        );
    }

    #[test]
    fn errors_are_clear() {
        assert!(parse("").is_err());
        assert!(parse("select 1").is_err());
        assert!(parse("use mydb").is_err());
        assert!(parse("db.c.find({a: })").is_err());
        assert!(parse("db.c.find({a: 1}").is_err());
        assert!(parse("db.c.find({a: 'x})").is_err());
        assert!(parse("db.c.bogus()").is_err());
        assert!(parse(r#"db.c.find({_id: ObjectId("zz")})"#).is_err());
        assert!(parse("db.c.find({}).explain()").is_err());
        assert!(parse("db.c.find({}) trailing").is_err());
    }
}
