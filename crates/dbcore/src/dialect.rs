//! Per-engine SQL syntax: quoting, literals, LIMIT/OFFSET, filter rendering.

use serde::{Deserialize, Serialize};

use crate::Engine;
use crate::error::{Error, Result};
use crate::model::{Filter, FilterOp, TableRef};
use crate::value::{CellInput, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dialect {
    Sqlite,
    Postgres,
    /// MySQL and MariaDB. `backslash_escapes` is false when sql_mode has NO_BACKSLASH_ESCAPES.
    Mysql {
        backslash_escapes: bool,
    },
    /// Mongo shell syntax (used by the splitter only).
    Mongo,
}

impl Dialect {
    pub fn for_engine(engine: Engine) -> Self {
        match engine {
            Engine::Sqlite => Dialect::Sqlite,
            Engine::Postgres => Dialect::Postgres,
            Engine::Mysql | Engine::Mariadb => Dialect::Mysql {
                backslash_escapes: true,
            },
            Engine::Mongodb => Dialect::Mongo,
        }
    }

    /// Quotes an identifier: `"a""b"` (Postgres/SQLite) or `` `a``b` `` (MySQL).
    pub fn quote_ident(&self, ident: &str) -> String {
        match self {
            Dialect::Mysql { .. } => format!("`{}`", ident.replace('`', "``")),
            _ => format!("\"{}\"", ident.replace('"', "\"\"")),
        }
    }

    /// Fully qualified, quoted table name (`"schema"."table"`, `` `db`.`table` ``,
    /// SQLite: `"db"."table"` unless db is "main").
    pub fn qualified(&self, table: &TableRef) -> String {
        match self {
            Dialect::Postgres => match &table.schema {
                Some(s) => format!("{}.{}", self.quote_ident(s), self.quote_ident(&table.name)),
                None => self.quote_ident(&table.name),
            },
            Dialect::Mysql { .. } => {
                if table.database.is_empty() {
                    self.quote_ident(&table.name)
                } else {
                    format!(
                        "{}.{}",
                        self.quote_ident(&table.database),
                        self.quote_ident(&table.name)
                    )
                }
            }
            Dialect::Sqlite => {
                if table.database.is_empty() || table.database == "main" {
                    self.quote_ident(&table.name)
                } else {
                    format!(
                        "{}.{}",
                        self.quote_ident(&table.database),
                        self.quote_ident(&table.name)
                    )
                }
            }
            Dialect::Mongo => table.name.clone(),
        }
    }

    /// SQL string literal with dialect-correct escaping.
    pub fn quote_string(&self, s: &str) -> String {
        if matches!(self, Dialect::Sqlite) && s.contains('\0') {
            // SQLite stops reading a statement at a NUL, so the text is joined from pieces.
            let pieces: Vec<String> = s
                .split('\0')
                .map(|piece| self.quote_string(piece))
                .collect();
            return format!("({})", pieces.join(" || char(0) || "));
        }
        let mut out = String::with_capacity(s.len() + 2);
        out.push('\'');
        match self {
            Dialect::Mysql {
                backslash_escapes: true,
            } => {
                for c in s.chars() {
                    match c {
                        '\'' => out.push_str("''"),
                        '\\' => out.push_str("\\\\"),
                        '\0' => out.push_str("\\0"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\u{1a}' => out.push_str("\\Z"),
                        c => out.push(c),
                    }
                }
            }
            _ => {
                for c in s.chars() {
                    if c == '\'' {
                        out.push('\'');
                    }
                    out.push(c);
                }
            }
        }
        out.push('\'');
        out
    }

    /// SQL literal for a loaded value (used in WHERE clauses of change sets and SQL export).
    pub fn literal(&self, value: &Value) -> Result<String> {
        Ok(match value {
            Value::Null => "NULL".to_owned(),
            Value::Bool(b) => match (self, b) {
                (Dialect::Sqlite, true) => "1".to_owned(),
                (Dialect::Sqlite, false) => "0".to_owned(),
                (_, true) => "TRUE".to_owned(),
                (_, false) => "FALSE".to_owned(),
            },
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) if f.is_finite() => float_literal(*f),
            Value::Float(f) => match self {
                Dialect::Postgres => {
                    let t = if f.is_nan() {
                        "NaN"
                    } else if *f > 0.0 {
                        "Infinity"
                    } else {
                        "-Infinity"
                    };
                    format!("'{t}'::float8")
                }
                Dialect::Sqlite if f.is_infinite() => {
                    if *f > 0.0 { "9e999" } else { "-9e999" }.to_owned()
                }
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "{} can't be written as a number in this database.",
                        if f.is_nan() { "NaN" } else { "Infinity" }
                    )));
                }
            },
            Value::Decimal(d) => {
                if is_plain_number(d) {
                    d.clone()
                } else {
                    // NaN, money ("$1.00"), anything the engine formatted specially.
                    self.quote_string(d)
                }
            }
            Value::Text(s) => self.quote_string(s),
            Value::TruncatedText { .. } => {
                return Err(Error::InvalidInput(
                    "The text is too long to be used here: it was shortened for display."
                        .to_owned(),
                ));
            }
            Value::Bytes { data, len } => {
                if data.len() != *len {
                    return Err(Error::InvalidInput(
                        "The binary value is too large to be used here: it was shortened for display."
                            .to_owned(),
                    ));
                }
                self.bytes_literal(data)
            }
            Value::Json(j) => self.quote_string(&j.to_string()),
            Value::Date(s)
            | Value::Time(s)
            | Value::DateTime(s)
            | Value::Uuid(s)
            | Value::ObjectId(s) => self.quote_string(s),
            Value::Other { type_name, text } => {
                // MySQL renders BIT values as b'0101', which is itself a literal.
                if matches!(self, Dialect::Mysql { .. })
                    && type_name.eq_ignore_ascii_case("bit")
                    && is_bit_literal(text)
                {
                    text.clone()
                } else {
                    self.quote_string(text)
                }
            }
        })
    }

    /// SQL literal for user input (`CellInput::Default` renders `DEFAULT`).
    pub fn input_literal(&self, input: &CellInput) -> Result<String> {
        Ok(match input {
            CellInput::Null => "NULL".to_owned(),
            CellInput::Default => "DEFAULT".to_owned(),
            CellInput::Text(s) => self.quote_string(s),
            CellInput::Bytes(b64) => {
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .map_err(|e| Error::InvalidInput(format!("Invalid binary value: {e}")))?;
                self.bytes_literal(&data)
            }
            CellInput::Json(j) => self.quote_string(&j.to_string()),
            CellInput::Original(v) => self.literal(v)?,
        })
    }

    /// Hex literal for binary data: `'\xdead'::bytea` (Postgres), `X'dead'` (MySQL, SQLite).
    pub fn bytes_literal(&self, data: &[u8]) -> String {
        match self {
            Dialect::Postgres => format!("'\\x{}'::bytea", hex::encode(data)),
            _ => format!("X'{}'", hex::encode(data)),
        }
    }

    /// `LIMIT n OFFSET m` (all three SQL dialects accept this form).
    pub fn limit_offset(&self, limit: u32, offset: u64) -> String {
        format!("LIMIT {limit} OFFSET {offset}")
    }

    /// Renders one filter as a boolean SQL expression.
    pub fn filter_sql(&self, filter: &Filter) -> Result<String> {
        if matches!(self, Dialect::Mongo) {
            return Err(Error::Unsupported(
                "SQL filters are not available for MongoDB.".to_owned(),
            ));
        }
        let col = self.quote_ident(&filter.column);
        let value = || -> Result<&str> {
            filter.value.as_deref().ok_or_else(|| {
                Error::InvalidInput(format!(
                    "The filter on \"{}\" needs a value.",
                    filter.column
                ))
            })
        };
        // Postgres can't LIKE a non-text column; cast, and use ILIKE for case-insensitivity.
        let (lhs, like, not_like) = match self {
            Dialect::Postgres => (format!("{col}::text"), "ILIKE", "NOT ILIKE"),
            _ => (col.clone(), "LIKE", "NOT LIKE"),
        };
        let esc = format!("ESCAPE {}", self.quote_string("\\"));
        let pattern = |prefix: &str, v: &str, suffix: &str| -> String {
            self.quote_string(&format!("{prefix}{}{suffix}", escape_like(v)))
        };
        Ok(match filter.op {
            FilterOp::Eq => format!("{col} = {}", self.quote_string(value()?)),
            FilterOp::Neq => format!("{col} <> {}", self.quote_string(value()?)),
            FilterOp::Lt => format!("{col} < {}", self.quote_string(value()?)),
            FilterOp::Lte => format!("{col} <= {}", self.quote_string(value()?)),
            FilterOp::Gt => format!("{col} > {}", self.quote_string(value()?)),
            FilterOp::Gte => format!("{col} >= {}", self.quote_string(value()?)),
            FilterOp::Contains => format!("{lhs} {like} {} {esc}", pattern("%", value()?, "%")),
            FilterOp::NotContains => {
                format!("{lhs} {not_like} {} {esc}", pattern("%", value()?, "%"))
            }
            FilterOp::StartsWith => format!("{lhs} {like} {} {esc}", pattern("", value()?, "%")),
            FilterOp::EndsWith => format!("{lhs} {like} {} {esc}", pattern("%", value()?, "")),
            FilterOp::Like => format!("{lhs} LIKE {}", self.quote_string(value()?)),
            FilterOp::In | FilterOp::NotIn => {
                let items = split_list(value()?);
                let negate = filter.op == FilterOp::NotIn;
                if items.is_empty() {
                    // `IN ()` is a syntax error; an empty set matches nothing.
                    if negate { "1 = 1" } else { "1 = 0" }.to_owned()
                } else {
                    let list: Vec<String> = items.iter().map(|i| self.quote_string(i)).collect();
                    format!(
                        "{col} {} ({})",
                        if negate { "NOT IN" } else { "IN" },
                        list.join(", ")
                    )
                }
            }
            FilterOp::IsNull => format!("{col} IS NULL"),
            FilterOp::IsNotNull => format!("{col} IS NOT NULL"),
        })
    }
}

/// A finite float as a SQL number. Rust's `Display` never writes an exponent, so 1e300
/// would be 301 digits (which MySQL reads as an out-of-range DECIMAL and rounds) and 1e-300 a
/// long run of zeros; extreme magnitudes are written in scientific notation instead.
fn float_literal(f: f64) -> String {
    if f != 0.0 && (f.abs() >= 1e15 || f.abs() < 1e-5) {
        format!("{f:e}")
    } else {
        f.to_string()
    }
}

/// Escapes `\`, `%` and `_` so the text matches literally inside a LIKE pattern.
pub fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Splits a comma-separated filter list. An item may be wrapped in single quotes
/// (`''` for a quote) to include commas; unquoted items are trimmed and empty ones dropped.
pub fn split_list(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut quoted = false; // current item started with a quote
    let mut in_quote = false;
    let mut chars = s.chars().peekable();
    let flush = |items: &mut Vec<String>, cur: &mut String, quoted: bool| {
        let item = if quoted {
            std::mem::take(cur)
        } else {
            std::mem::take(cur).trim().to_owned()
        };
        if quoted || !item.is_empty() {
            items.push(item);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' if in_quote => {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    cur.push('\'');
                } else {
                    in_quote = false;
                }
            }
            '\'' if !quoted && cur.trim().is_empty() => {
                in_quote = true;
                quoted = true;
                cur.clear();
            }
            ',' if !in_quote => {
                flush(&mut items, &mut cur, quoted);
                quoted = false;
            }
            // Whitespace after a closing quote is padding, not part of the value.
            c if quoted && !in_quote && c.is_whitespace() => {}
            c => cur.push(c),
        }
    }
    flush(&mut items, &mut cur, quoted);
    items
}

/// True for `-12`, `3.14`, `.5`, `1e10`, `+7`: text safe to emit unquoted as a number.
pub fn is_plain_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if i < b.len() && matches!(b[i], b'e' | b'E') {
        i += 1;
        if i < b.len() && matches!(b[i], b'-' | b'+') {
            i += 1;
        }
        let exp = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp {
            return false;
        }
    }
    i == b.len()
}

fn is_bit_literal(s: &str) -> bool {
    s.len() >= 3
        && (s.starts_with("b'") || s.starts_with("B'"))
        && s.ends_with('\'')
        && s[2..s.len() - 1].bytes().all(|c| c == b'0' || c == b'1')
}

#[cfg(test)]
mod tests {
    use super::*;

    const MY: Dialect = Dialect::Mysql {
        backslash_escapes: true,
    };
    const MY_NB: Dialect = Dialect::Mysql {
        backslash_escapes: false,
    };

    fn filter(op: FilterOp, value: Option<&str>) -> Filter {
        Filter {
            column: "name".into(),
            op,
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn quotes_strings_per_dialect() {
        assert_eq!(Dialect::Postgres.quote_string("it's"), "'it''s'");
        assert_eq!(Dialect::Sqlite.quote_string("a\\b"), "'a\\b'");
        assert_eq!(MY.quote_string("it's"), "'it''s'");
        assert_eq!(MY.quote_string("a\\b\0\n\r\u{1a}"), "'a\\\\b\\0\\n\\r\\Z'");
        assert_eq!(MY_NB.quote_string("a\\b'"), "'a\\b'''");
        assert_eq!(MY.quote_string("日本語"), "'日本語'");
    }

    #[test]
    fn sqlite_strings_with_nul_are_joined_from_pieces() {
        assert_eq!(
            Dialect::Sqlite.quote_string("a\0b'c\0"),
            "('a' || char(0) || 'b''c' || char(0) || '')"
        );
        assert_eq!(Dialect::Sqlite.quote_string("plain"), "'plain'");
    }

    #[test]
    fn extreme_floats_use_scientific_notation() {
        let lit = |f: f64| Dialect::Postgres.literal(&Value::Float(f)).unwrap();
        assert_eq!(lit(0.0), "0");
        assert_eq!(lit(1.5), "1.5");
        assert_eq!(lit(-0.1), "-0.1");
        assert_eq!(lit(100.0), "100");
        assert_eq!(lit(123456789.125), "123456789.125");
        assert_eq!(lit(f64::MAX), "1.7976931348623157e308");
        assert_eq!(lit(5e-324), "5e-324");
        assert_eq!(lit(1e-300), "1e-300");
        assert_eq!(lit(-2.5e20), "-2.5e20");
        // Every literal reads back as the same number.
        for f in [f64::MAX, f64::MIN_POSITIVE, 1e15, 1e-5, 0.3, 123456.789e10] {
            assert_eq!(lit(f).parse::<f64>().unwrap(), f);
        }
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(Dialect::Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(MY.quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn literals() {
        let pg = Dialect::Postgres;
        assert_eq!(pg.literal(&Value::Null).unwrap(), "NULL");
        assert_eq!(pg.literal(&Value::Bool(true)).unwrap(), "TRUE");
        assert_eq!(Dialect::Sqlite.literal(&Value::Bool(true)).unwrap(), "1");
        assert_eq!(pg.literal(&Value::Int(-5)).unwrap(), "-5");
        assert_eq!(
            pg.literal(&Value::UInt(u64::MAX)).unwrap(),
            "18446744073709551615"
        );
        assert_eq!(pg.literal(&Value::Float(1.5)).unwrap(), "1.5");
        assert_eq!(
            pg.literal(&Value::Float(f64::NAN)).unwrap(),
            "'NaN'::float8"
        );
        assert!(MY.literal(&Value::Float(f64::INFINITY)).is_err());
        assert_eq!(
            Dialect::Sqlite
                .literal(&Value::Float(f64::NEG_INFINITY))
                .unwrap(),
            "-9e999"
        );
        assert_eq!(
            pg.literal(&Value::Decimal("12.50".into())).unwrap(),
            "12.50"
        );
        assert_eq!(pg.literal(&Value::Decimal("NaN".into())).unwrap(), "'NaN'");
        assert_eq!(
            pg.literal(&Value::Decimal("$19.99".into())).unwrap(),
            "'$19.99'"
        );
        assert_eq!(
            pg.literal(&Value::bytes(vec![0xde, 0xad])).unwrap(),
            "'\\xdead'::bytea"
        );
        assert_eq!(
            MY.literal(&Value::bytes(vec![0xde, 0xad])).unwrap(),
            "X'dead'"
        );
        assert_eq!(
            Dialect::Sqlite.literal(&Value::bytes(vec![])).unwrap(),
            "X''"
        );
        assert_eq!(
            pg.literal(&Value::Json(serde_json::json!({"a": "it's"})))
                .unwrap(),
            "'{\"a\":\"it''s\"}'"
        );
        assert_eq!(
            pg.literal(&Value::DateTime("2024-01-01 00:00:00".into()))
                .unwrap(),
            "'2024-01-01 00:00:00'"
        );
        assert_eq!(
            MY.literal(&Value::Other {
                type_name: "BIT".into(),
                text: "b'101'".into()
            })
            .unwrap(),
            "b'101'"
        );
        assert_eq!(
            pg.literal(&Value::Other {
                type_name: "int4range".into(),
                text: "[1,10)".into()
            })
            .unwrap(),
            "'[1,10)'"
        );
    }

    #[test]
    fn partial_values_are_rejected() {
        let t = Value::TruncatedText {
            prefix: "abc".into(),
            len: 100_000,
        };
        assert!(matches!(
            Dialect::Postgres.literal(&t),
            Err(Error::InvalidInput(_))
        ));
        let b = Value::Bytes {
            data: vec![1],
            len: 5000,
        };
        assert!(matches!(MY.literal(&b), Err(Error::InvalidInput(_))));
    }

    #[test]
    fn input_literals() {
        let d = Dialect::Postgres;
        assert_eq!(d.input_literal(&CellInput::Default).unwrap(), "DEFAULT");
        assert_eq!(d.input_literal(&CellInput::Null).unwrap(), "NULL");
        assert_eq!(
            d.input_literal(&CellInput::Text("x'y".into())).unwrap(),
            "'x''y'"
        );
        assert_eq!(
            d.input_literal(&CellInput::Bytes("3q0=".into())).unwrap(),
            "'\\xdead'::bytea"
        );
        assert!(d.input_literal(&CellInput::Bytes("!!".into())).is_err());
        assert_eq!(
            d.input_literal(&CellInput::Json(serde_json::json!([1, 2])))
                .unwrap(),
            "'[1,2]'"
        );
        assert_eq!(
            d.input_literal(&CellInput::Original(Value::Int(3)))
                .unwrap(),
            "3"
        );
    }

    #[test]
    fn filters() {
        let pg = Dialect::Postgres;
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::Eq, Some("a'b"))).unwrap(),
            "\"name\" = 'a''b'"
        );
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::Contains, Some("50%_x")))
                .unwrap(),
            "\"name\"::text ILIKE '%50\\%\\_x%' ESCAPE '\\'"
        );
        assert_eq!(
            MY.filter_sql(&filter(FilterOp::StartsWith, Some("ab")))
                .unwrap(),
            "`name` LIKE 'ab%' ESCAPE '\\\\'"
        );
        assert_eq!(
            MY_NB
                .filter_sql(&filter(FilterOp::EndsWith, Some("ab")))
                .unwrap(),
            "`name` LIKE '%ab' ESCAPE '\\'"
        );
        assert_eq!(
            Dialect::Sqlite
                .filter_sql(&filter(FilterOp::NotContains, Some("x")))
                .unwrap(),
            "\"name\" NOT LIKE '%x%' ESCAPE '\\'"
        );
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::In, Some("1, 2,'a,b'")))
                .unwrap(),
            "\"name\" IN ('1', '2', 'a,b')"
        );
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::NotIn, Some(""))).unwrap(),
            "1 = 1"
        );
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::IsNull, Some("ignored")))
                .unwrap(),
            "\"name\" IS NULL"
        );
        assert_eq!(
            pg.filter_sql(&filter(FilterOp::IsNotNull, None)).unwrap(),
            "\"name\" IS NOT NULL"
        );
        assert!(pg.filter_sql(&filter(FilterOp::Eq, None)).is_err());
    }

    #[test]
    fn list_splitting() {
        assert_eq!(split_list("a, b ,c"), ["a", "b", "c"]);
        assert_eq!(split_list("'a, b', c"), ["a, b", "c"]);
        assert_eq!(split_list("'it''s'"), ["it's"]);
        assert_eq!(split_list("  "), Vec::<String>::new());
        assert_eq!(split_list("a,,b"), ["a", "b"]);
        assert_eq!(split_list("''"), [""]);
        assert_eq!(split_list("'a' , 'b'"), ["a", "b"]);
    }

    #[test]
    fn plain_numbers() {
        for ok in [
            "0",
            "-1",
            "+2.5",
            ".5",
            "1e10",
            "1.5E-3",
            "123456789012345678901234567890.5",
        ] {
            assert!(is_plain_number(ok), "{ok}");
        }
        for bad in [
            "", "-", ".", "NaN", "$1.00", "1e", "1,000", "0x10", "Infinity",
        ] {
            assert!(!is_plain_number(bad), "{bad}");
        }
    }
}
