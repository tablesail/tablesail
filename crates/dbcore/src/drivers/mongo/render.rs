//! Renders BSON as mongosh source text (what the UI shows as the "query" and
//! what change previews print). The output parses back with `shell::parse_value`.

use base64::Engine as _;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Bson, Document};

use super::convert::format_millis;
use crate::model::TableAction;

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn quote(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

fn key(k: &str) -> String {
    if is_identifier(k) {
        k.to_owned()
    } else {
        quote(k)
    }
}

/// `/pattern/flags`, with unescaped slashes escaped so the literal stays valid.
fn regex_literal(pattern: &str, options: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 2);
    let mut escaped = false;
    let mut in_class = false;
    for c in pattern.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => out.push('\\'),
            _ => {}
        }
        out.push(c);
    }
    format!("/{out}/{options}")
}

pub fn render(bson: &Bson) -> String {
    match bson {
        Bson::Document(d) => render_doc(d),
        Bson::Array(items) if items.is_empty() => "[]".into(),
        Bson::Array(items) => format!(
            "[ {} ]",
            items.iter().map(render).collect::<Vec<_>>().join(", ")
        ),
        Bson::String(s) | Bson::Symbol(s) => quote(s),
        Bson::Int32(i) => i.to_string(),
        Bson::Int64(i) => format!("NumberLong(\"{i}\")"),
        Bson::Double(f) if f.is_nan() => "NaN".into(),
        Bson::Double(f) if f.is_infinite() => {
            if *f > 0.0 { "Infinity" } else { "-Infinity" }.into()
        }
        Bson::Double(f) => {
            let s = f.to_string();
            if s.contains(['.', 'e', 'E']) {
                s
            } else {
                format!("{s}.0")
            }
        }
        Bson::Decimal128(d) => format!("NumberDecimal(\"{d}\")"),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".into(),
        Bson::Undefined => "undefined".into(),
        Bson::ObjectId(o) => format!("ObjectId(\"{}\")", o.to_hex()),
        Bson::DateTime(d) => match format_millis(d.timestamp_millis()) {
            Some(s) => format!("ISODate(\"{s}\")"),
            None => format!("new Date({})", d.timestamp_millis()),
        },
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&b.bytes);
            format!("UUID(\"{}\")", uuid::Uuid::from_bytes(raw).hyphenated())
        }
        Bson::Binary(b) => format!(
            "BinData({}, \"{}\")",
            u8::from(b.subtype),
            base64::engine::general_purpose::STANDARD.encode(&b.bytes)
        ),
        Bson::RegularExpression(r) => regex_literal(&r.pattern, &r.options),
        Bson::Timestamp(t) => format!("Timestamp({}, {})", t.time, t.increment),
        Bson::MinKey => "MinKey()".into(),
        Bson::MaxKey => "MaxKey()".into(),
        Bson::JavaScriptCode(c) => quote(c),
        Bson::JavaScriptCodeWithScope(c) => quote(&c.code),
        Bson::DbPointer(_) => quote(&bson.to_string()),
    }
}

pub fn render_doc(doc: &Document) -> String {
    if doc.is_empty() {
        return "{}".into();
    }
    let fields: Vec<String> = doc
        .iter()
        .map(|(k, v)| format!("{}: {}", key(k), render(v)))
        .collect();
    format!("{{ {} }}", fields.join(", "))
}

/// `db.users` or `db.getCollection("my-coll")`.
pub fn collection_ref(name: &str) -> String {
    if !name.is_empty() && name.split('.').all(is_identifier) {
        format!("db.{name}")
    } else {
        format!("db.getCollection({})", quote(name))
    }
}

/// The shell command that drops, empties or renames a collection.
pub fn admin_command(collection: &str, action: &TableAction) -> String {
    let target = collection_ref(collection);
    match action {
        TableAction::Drop => format!("{target}.drop()"),
        TableAction::Truncate => format!("{target}.deleteMany({{}})"),
        TableAction::Rename { new_name } => {
            format!("{target}.renameCollection({})", quote(new_name))
        }
    }
}

/// A find as shell text, e.g. `db.users.find({ a: 1 }).sort({ _id: 1 }).skip(0).limit(100)`.
pub fn render_find(
    collection: &str,
    filter: &Document,
    projection: Option<&Document>,
    sort: Option<&Document>,
    skip: u64,
    limit: Option<i64>,
) -> String {
    let mut out = format!("{}.find({}", collection_ref(collection), render_doc(filter));
    if let Some(p) = projection.filter(|p| !p.is_empty()) {
        out.push_str(&format!(", {}", render_doc(p)));
    }
    out.push(')');
    if let Some(s) = sort.filter(|s| !s.is_empty()) {
        out.push_str(&format!(".sort({})", render_doc(s)));
    }
    if skip > 0 {
        out.push_str(&format!(".skip({skip})"));
    }
    if let Some(l) = limit.filter(|l| *l > 0) {
        out.push_str(&format!(".limit({l})"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::mongo::shell::parse_value;
    use mongodb::bson::doc;

    #[test]
    fn rendering_round_trips() {
        let original = Bson::Document(doc! {
            "_id": mongodb::bson::oid::ObjectId::parse_str("65f000000000000000000001").unwrap(),
            "name": "O'Brien \"x\"",
            "n": 5i64,
            "d": mongodb::bson::Decimal128::from_bytes([0; 16]),
            "f": 2.0,
            "at": mongodb::bson::DateTime::from_millis(1_714_566_896_789),
            "re": mongodb::bson::Regex { pattern: "a/b".into(), options: "i".into() },
            "weird key": [1, "two", { "x": null }],
            "$or": [{ "a": true }],
        });
        let text = render(&original);
        assert_eq!(parse_value(&text).unwrap(), original, "{text}");
    }

    #[test]
    fn find_text() {
        let text = render_find(
            "users",
            &doc! { "age": { "$gt": 18 } },
            None,
            Some(&doc! { "_id": 1 }),
            0,
            Some(100),
        );
        assert_eq!(
            text,
            "db.users.find({ age: { $gt: 18 } }).sort({ _id: 1 }).limit(100)"
        );
        assert_eq!(collection_ref("my-coll"), "db.getCollection(\"my-coll\")");
        assert_eq!(collection_ref("system.users"), "db.system.users");
    }
}
