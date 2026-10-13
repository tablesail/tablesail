//! `BrowseRequest` -> Mongo filter, sort and projection documents.

use mongodb::bson::{Bson, Document, Regex, doc};

use super::convert::{Target, parse_datetime, text_to_bson};
use super::shell;
use crate::error::Result;
use crate::model::{BrowseRequest, ColumnInfo, Filter, FilterOp, TypeFamily};

fn column<'a>(columns: &'a [ColumnInfo], name: &str) -> Option<&'a ColumnInfo> {
    columns.iter().find(|c| c.name == name)
}

/// A filter value typed by the column's family; text that does not parse as
/// that type stays a string.
fn typed(columns: &[ColumnInfo], name: &str, text: &str) -> Bson {
    let target = match column(columns, name) {
        Some(c) => Target::from_column(c),
        None => Target::Unknown,
    };
    // Dates need to be real dates to compare; everything else goes through the
    // same conversion cell edits use.
    if target == Target::DateTime && parse_datetime(text).is_none() {
        return Bson::String(text.to_owned());
    }
    match target {
        // A document/array column compared with plain text means "contains this string".
        Target::Container => Bson::String(text.to_owned()),
        t => text_to_bson(text, t).unwrap_or_else(|_| Bson::String(text.to_owned())),
    }
}

/// Escapes regex metacharacters.
pub fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// SQL LIKE pattern -> anchored regex (`%` any run, `_` any char, `\` escapes).
pub fn like_to_regex(pattern: &str) -> String {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next) => out.push_str(&escape_regex(&next.to_string())),
                None => out.push_str("\\\\"),
            },
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            other => out.push_str(&escape_regex(&other.to_string())),
        }
    }
    out.push('$');
    out
}

fn regex(pattern: String) -> Bson {
    Bson::RegularExpression(Regex {
        pattern,
        options: "is".into(),
    })
}

fn needs_stringify(family: Option<TypeFamily>) -> bool {
    matches!(
        family,
        Some(
            TypeFamily::Integer
                | TypeFamily::Float
                | TypeFamily::Decimal
                | TypeFamily::ObjectId
                | TypeFamily::Date
                | TypeFamily::DateTime
                | TypeFamily::Boolean
        )
    )
}

/// Pattern match on a field; scalar non-text columns are compared as text so
/// "contains 12" works on numbers and ObjectIds too.
fn pattern_match(
    field: &str,
    pattern: String,
    negate: bool,
    family: Option<TypeFamily>,
) -> Document {
    if needs_stringify(family) {
        let matched = doc! { "$regexMatch": {
            "input": { "$convert": {
                "input": format!("${field}"),
                "to": "string",
                "onError": "",
                "onNull": "",
            }},
            "regex": pattern,
            "options": "is",
        }};
        let expr = if negate {
            doc! { "$not": [matched] }
        } else {
            matched.clone()
        };
        return doc! { "$expr": expr };
    }
    if negate {
        doc! { field: { "$not": regex(pattern) } }
    } else {
        doc! { field: regex(pattern) }
    }
}

fn list_items(text: &str) -> Vec<&str> {
    text.split(',')
        .map(str::trim)
        .map(|item| {
            let quoted = item.len() >= 2
                && ((item.starts_with('\'') && item.ends_with('\''))
                    || (item.starts_with('"') && item.ends_with('"')));
            if quoted {
                &item[1..item.len() - 1]
            } else {
                item
            }
        })
        .collect()
}

fn condition(columns: &[ColumnInfo], f: &Filter) -> Document {
    let name = f.column.as_str();
    let text = f.value.as_deref().unwrap_or("");
    let family = column(columns, name).map(|c| c.family);
    let value = || typed(columns, name, text);
    match f.op {
        FilterOp::Eq => doc! { name: value() },
        FilterOp::Neq => doc! { name: { "$ne": value() } },
        FilterOp::Lt => doc! { name: { "$lt": value() } },
        FilterOp::Lte => doc! { name: { "$lte": value() } },
        FilterOp::Gt => doc! { name: { "$gt": value() } },
        FilterOp::Gte => doc! { name: { "$gte": value() } },
        FilterOp::Contains => pattern_match(name, escape_regex(text), false, family),
        FilterOp::NotContains => pattern_match(name, escape_regex(text), true, family),
        FilterOp::StartsWith => {
            pattern_match(name, format!("^{}", escape_regex(text)), false, family)
        }
        FilterOp::EndsWith => {
            pattern_match(name, format!("{}$", escape_regex(text)), false, family)
        }
        FilterOp::Like => pattern_match(name, like_to_regex(text), false, family),
        FilterOp::In | FilterOp::NotIn => {
            let items: Vec<Bson> = list_items(text)
                .into_iter()
                .map(|i| typed(columns, name, i))
                .collect();
            let op = if f.op == FilterOp::In { "$in" } else { "$nin" };
            doc! { name: { op: items } }
        }
        FilterOp::IsNull => doc! { name: Bson::Null },
        FilterOp::IsNotNull => doc! { name: { "$ne": Bson::Null } },
    }
}

/// The query document for a request: filters joined by `match_all`, ANDed with `raw_where`.
pub fn build_filter(request: &BrowseRequest, columns: &[ColumnInfo]) -> Result<Document> {
    let mut conditions: Vec<Document> = request
        .filters
        .iter()
        .map(|f| condition(columns, f))
        .collect();
    let joined = match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => {
            let key = if request.match_all { "$and" } else { "$or" };
            Some(doc! { key: conditions })
        }
    };
    let raw = match request.raw_where.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => Some(shell::parse_document(text)?),
        _ => None,
    }
    .filter(|d| !d.is_empty());
    Ok(match (joined, raw) {
        (None, None) => Document::new(),
        (Some(a), None) | (None, Some(a)) => a,
        (Some(a), Some(b)) => doc! { "$and": [a, b] },
    })
}

/// Requested sort, defaulting to `_id` ascending; `_id` is appended as a
/// tie-breaker so pages stay stable.
pub fn build_sort(request: &BrowseRequest) -> Document {
    let mut sort = Document::new();
    for s in &request.sort {
        sort.insert(s.column.clone(), if s.descending { -1 } else { 1 });
    }
    if !sort.contains_key("_id") {
        sort.insert("_id", 1);
    }
    sort
}

/// Projection for `request.columns`; `_id` is always kept (editing needs it).
pub fn build_projection(request: &BrowseRequest) -> Option<Document> {
    if request.columns.is_empty() {
        return None;
    }
    let mut projection = doc! { "_id": 1 };
    for c in &request.columns {
        projection.insert(c.clone(), 1);
    }
    Some(projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Sort, TableRef};

    fn col(name: &str, family: TypeFamily) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: String::new(),
            family,
            nullable: true,
            default: None,
            is_primary_key: false,
            is_auto_increment: false,
            is_generated: false,
            enum_values: None,
            comment: None,
        }
    }

    fn request(filters: Vec<Filter>) -> BrowseRequest {
        BrowseRequest {
            table: TableRef {
                database: "d".into(),
                schema: None,
                name: "c".into(),
            },
            filters,
            match_all: true,
            raw_where: None,
            sort: vec![],
            columns: vec![],
        }
    }

    fn f(column: &str, op: FilterOp, value: Option<&str>) -> Filter {
        Filter {
            column: column.into(),
            op,
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn values_follow_the_column_family() {
        let cols = [
            col("age", TypeFamily::Integer),
            col("_id", TypeFamily::ObjectId),
            col("name", TypeFamily::Text),
        ];
        let d = build_filter(
            &request(vec![
                f("age", FilterOp::Gt, Some("18")),
                f("_id", FilterOp::Eq, Some("65f000000000000000000001")),
                f("name", FilterOp::Eq, Some("42")),
            ]),
            &cols,
        )
        .unwrap();
        let and = d.get_array("$and").unwrap();
        assert_eq!(
            and[0].as_document().unwrap(),
            &doc! { "age": { "$gt": 18 } }
        );
        assert!(matches!(
            and[1].as_document().unwrap().get("_id"),
            Some(Bson::ObjectId(_))
        ));
        assert_eq!(and[2].as_document().unwrap(), &doc! { "name": "42" });
    }

    #[test]
    fn text_operators_use_escaped_regexes() {
        let cols = [col("name", TypeFamily::Text), col("n", TypeFamily::Integer)];
        let d = build_filter(
            &request(vec![f("name", FilterOp::Contains, Some("a.b"))]),
            &cols,
        )
        .unwrap();
        assert_eq!(
            d.get("name"),
            Some(&Bson::RegularExpression(Regex {
                pattern: "a\\.b".into(),
                options: "is".into()
            }))
        );
        let d = build_filter(
            &request(vec![f("n", FilterOp::StartsWith, Some("12"))]),
            &cols,
        )
        .unwrap();
        assert!(d.contains_key("$expr"));
        assert_eq!(like_to_regex("a%b_c\\%"), "^a.*b.c%$");
    }

    #[test]
    fn lists_nulls_and_or() {
        let cols = [col("n", TypeFamily::Integer)];
        let mut r = request(vec![
            f("n", FilterOp::In, Some("1, 2,3")),
            f("n", FilterOp::IsNull, None),
        ]);
        r.match_all = false;
        r.raw_where = Some("{ extra: true }".into());
        let d = build_filter(&r, &cols).unwrap();
        let and = d.get_array("$and").unwrap();
        let or = and[0].as_document().unwrap().get_array("$or").unwrap();
        assert_eq!(
            or[0].as_document().unwrap(),
            &doc! { "n": { "$in": [1, 2, 3] } }
        );
        assert_eq!(or[1].as_document().unwrap(), &doc! { "n": Bson::Null });
        assert_eq!(and[1].as_document().unwrap(), &doc! { "extra": true });
    }

    #[test]
    fn sort_defaults_to_id() {
        assert_eq!(build_sort(&request(vec![])), doc! { "_id": 1 });
        let mut r = request(vec![]);
        r.sort = vec![Sort {
            column: "age".into(),
            descending: true,
        }];
        assert_eq!(build_sort(&r), doc! { "age": -1, "_id": 1 });
    }
}
