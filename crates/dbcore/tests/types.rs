//! Value round-trips for every type family on every SQL engine.
//!
//! Each case creates a one-column scratch table, inserts literals through the side
//! channel, then reads them back through every read path of the driver (`browse_page`,
//! `browse_all` and `query`) and compares the typed `Value`. Then every value is
//! written back with a change set, both as the loaded value (`CellInput::Original`)
//! and as typed text, and read again: what the grid shows must be what the database
//! stores, and what a user types back must be accepted and unchanged.

#[macro_use]
mod common;

use common::*;
use dbcore::model::{ChangeSet, ColumnInput, ColumnValue, RowChange, TypeFamily};
use dbcore::value::CellInput;
use dbcore::{Engine, Value};

// ---------------------------------------------------------------------------
// Case machinery
// ---------------------------------------------------------------------------

/// How a loaded value is typed back in for the second edit round.
#[derive(Clone, Copy, PartialEq)]
enum Typed {
    /// `to_display()` text.
    Text,
    /// Raw bytes as base64.
    Bytes,
    /// A JSON document.
    Json,
    /// Booleans are typed as `1`/`0` (what the cell editor sends to MySQL and SQLite).
    BoolNum,
    /// Not editable as text (skip this round).
    No,
}

struct Case {
    ty: String,
    /// Expected `ColumnMeta.family` (None: not asserted).
    family: Option<TypeFamily>,
    /// SQL literal and the value the driver must produce for it.
    rows: Vec<(String, Value)>,
    typed: Typed,
}

fn case(ty: &str, family: TypeFamily, typed: Typed, rows: Vec<(&str, Value)>) -> Case {
    Case {
        ty: ty.to_owned(),
        family: Some(family),
        rows: rows.into_iter().map(|(l, v)| (l.to_owned(), v)).collect(),
        typed,
    }
}

fn int(i: i64) -> Value {
    Value::Int(i)
}
fn text(s: &str) -> Value {
    Value::Text(s.to_owned())
}
fn dec(s: &str) -> Value {
    Value::Decimal(s.to_owned())
}
fn float(f: f64) -> Value {
    Value::Float(f)
}
fn date(s: &str) -> Value {
    Value::Date(s.to_owned())
}
fn time(s: &str) -> Value {
    Value::Time(s.to_owned())
}
fn datetime(s: &str) -> Value {
    Value::DateTime(s.to_owned())
}
fn json(j: serde_json::Value) -> Value {
    Value::Json(j)
}
fn bytes(b: &[u8]) -> Value {
    Value::bytes(b.to_vec())
}
fn other(ty: &str, text: &str) -> Value {
    Value::Other {
        type_name: ty.to_owned(),
        text: text.to_owned(),
    }
}

/// Equality that treats NaN as equal to NaN.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => (x.is_nan() && y.is_nan()) || x == y,
        _ => a == b,
    }
}

fn id_type(c: &Ctx) -> &'static str {
    match c.engine {
        Engine::Postgres => "integer",
        Engine::Sqlite => "INTEGER",
        _ => "INT",
    }
}

/// Runs every case; collects the problems of all of them before failing.
async fn run_cases(c: &Ctx, cases: Vec<Case>) {
    let mut problems = Vec::new();
    for (n, case) in cases.iter().enumerate() {
        if let Err(msg) = run_case(c, n, case).await {
            problems.push(format!("[{}] {msg}", case.ty));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} type cases failed:\n{}",
        problems.len(),
        cases.len(),
        problems.join("\n")
    );
}

macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}

async fn run_case(c: &Ctx, n: usize, case: &Case) -> Result<(), String> {
    let name = c.scratch(&format!("ty{n}"));
    let t = c.qt(&name);
    c.try_side_exec(&format!(
        "CREATE TABLE {t} (id {} PRIMARY KEY, v {})",
        id_type(c),
        case.ty
    ))
    .await
    .map_err(|e| format!("create: {e}"))?;
    for (i, (literal, _)) in case.rows.iter().enumerate() {
        c.try_side_exec(&format!(
            "INSERT INTO {t} (id, v) VALUES ({}, {literal})",
            i + 1
        ))
        .await
        .map_err(|e| format!("insert {literal}: {e}"))?;
    }
    let null_id = case.rows.len() + 1;
    c.try_side_exec(&format!("INSERT INTO {t} (id, v) VALUES ({null_id}, NULL)"))
        .await
        .map_err(|e| format!("insert NULL: {e}"))?;

    let mut expected: Vec<Value> = case.rows.iter().map(|(_, v)| v.clone()).collect();
    expected.push(Value::Null);

    // 1. The grid page.
    let req = c.browse(&name);
    let page = c
        .driver
        .browse_page(&req, 0, 100)
        .await
        .map_err(|e| format!("browse_page: {e:?}"))?;
    ensure!(page.columns.len() == 2, "columns: {:?}", page.columns);
    if let Some(family) = case.family {
        ensure!(
            page.columns[1].family == family,
            "family {:?} (type name {:?}), expected {family:?}",
            page.columns[1].family,
            page.columns[1].type_name
        );
    }
    let got: Vec<Value> = page.rows.iter().map(|r| r[1].clone()).collect();
    for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
        ensure!(
            same(g, e),
            "browse_page row {} ({}): got {g:?}, expected {e:?}",
            i + 1,
            case.rows.get(i).map_or("NULL", |r| r.0.as_str())
        );
    }
    ensure!(
        got.len() == expected.len(),
        "browse_page returned {} rows",
        got.len()
    );

    // 2. The export stream and the editor's query path give the same values.
    let all = c.all_rows(&req).await;
    let mut all: Vec<Row> = all;
    all.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    for (i, (r, e)) in all.iter().zip(&expected).enumerate() {
        ensure!(
            same(&r[1], e),
            "browse_all row {}: got {:?}, expected {e:?}",
            i + 1,
            r[1]
        );
    }
    let events = c
        .events(&format!("SELECT id, v FROM {t} ORDER BY id"))
        .await
        .map_err(|e| format!("query: {e:?}"))?;
    let set = &sets_of(&events)[0];
    for (i, (r, e)) in set.rows.iter().zip(&expected).enumerate() {
        ensure!(
            same(&r[1], e),
            "query row {}: got {:?}, expected {e:?}",
            i + 1,
            r[1]
        );
    }
    ensure!(
        set.columns[1].family == page.columns[1].family
            || c.is_sqlite()
            || set.columns[1].family == TypeFamily::Other,
        "query family {:?} differs from browse family {:?}",
        set.columns[1].family,
        page.columns[1].family
    );

    // 3. Write every value back: first as the loaded value, then as typed text.
    for round in 0..2 {
        if round == 1 && case.typed == Typed::No {
            continue;
        }
        for (i, loaded) in got.iter().enumerate() {
            let id = (i + 1) as i64;
            let input = if round == 0 {
                CellInput::Original(loaded.clone())
            } else {
                match (case.typed, loaded) {
                    (_, Value::Null) => CellInput::Null,
                    (Typed::Bytes, Value::Bytes { data, .. }) => {
                        use base64::Engine as _;
                        CellInput::Bytes(base64::engine::general_purpose::STANDARD.encode(data))
                    }
                    (Typed::Json, Value::Json(j)) => CellInput::Json(j.clone()),
                    (Typed::BoolNum, Value::Bool(b)) => {
                        CellInput::Text(if *b { "1" } else { "0" }.to_owned())
                    }
                    (_, v) => CellInput::Text(v.to_display()),
                }
            };
            // Start from a different value so the write is a real change.
            c.try_side_exec(&format!("UPDATE {t} SET v = NULL WHERE id = {id}"))
                .await
                .map_err(|e| format!("reset: {e}"))?;
            let changes = ChangeSet {
                table: c.tref(&name),
                changes: vec![RowChange::Update {
                    key: vec![ColumnValue {
                        column: "id".into(),
                        value: int(id),
                    }],
                    original: vec![ColumnValue {
                        column: "v".into(),
                        value: Value::Null,
                    }],
                    set: vec![ColumnInput {
                        column: "v".into(),
                        value: input.clone(),
                    }],
                }],
            };
            let what = format!("round {round} row {id} {input:?}");
            c.driver
                .preview_changes(&changes)
                .await
                .map_err(|e| format!("preview {what}: {e:?}"))?;
            let applied = c
                .driver
                .apply_changes(&changes)
                .await
                .map_err(|e| format!("apply {what}: {e:?}"))?;
            ensure!(applied.rows_affected == 1, "{what}: {applied:?}");
            let back = c
                .driver
                .browse_page(&req, (id - 1) as u64, 1)
                .await
                .map_err(|e| format!("re-read: {e:?}"))?;
            ensure!(
                same(&back.rows[0][1], &expected[i]),
                "{what}: stored {:?}, expected {:?}",
                back.rows[0][1],
                expected[i]
            );

            // The value just loaded is a valid optimistic-concurrency `original`.
            let next = ChangeSet {
                table: c.tref(&name),
                changes: vec![RowChange::Update {
                    key: vec![ColumnValue {
                        column: "id".into(),
                        value: int(id),
                    }],
                    original: vec![ColumnValue {
                        column: "v".into(),
                        value: back.rows[0][1].clone(),
                    }],
                    set: vec![ColumnInput {
                        column: "v".into(),
                        value: CellInput::Null,
                    }],
                }],
            };
            c.driver
                .apply_changes(&next)
                .await
                .map_err(|e| format!("original check after {what}: {e:?}"))?;
            let nulled = c
                .driver
                .browse_page(&req, (id - 1) as u64, 1)
                .await
                .map_err(|e| format!("re-read: {e:?}"))?;
            ensure!(
                nulled.rows[0][1] == Value::Null,
                "{what}: setting NULL left {:?}",
                nulled.rows[0][1]
            );
            // Put it back for the next round.
            c.driver
                .apply_changes(&ChangeSet {
                    table: c.tref(&name),
                    changes: vec![RowChange::Update {
                        key: vec![ColumnValue {
                            column: "id".into(),
                            value: int(id),
                        }],
                        original: vec![ColumnValue {
                            column: "v".into(),
                            value: Value::Null,
                        }],
                        set: vec![ColumnInput {
                            column: "v".into(),
                            value: CellInput::Original(loaded.clone()),
                        }],
                    }],
                })
                .await
                .map_err(|e| format!("restore {what}: {e:?}"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

fn long_unicode() -> String {
    "ünï çödé 日本語 العربية שלום 🎉 👨‍👩‍👧‍👦 e\u{301} \u{200b}\u{feff}\u{2028}".to_owned()
}

async fn pg_scalar_types(c: &Ctx) {
    use TypeFamily as F;
    let cases = vec![
        case(
            "smallint",
            F::Integer,
            Typed::Text,
            vec![
                ("32767", int(32767)),
                ("-32768", int(-32768)),
                ("0", int(0)),
            ],
        ),
        case(
            "integer",
            F::Integer,
            Typed::Text,
            vec![
                ("2147483647", int(i32::MAX as i64)),
                ("-2147483648", int(i32::MIN as i64)),
            ],
        ),
        case(
            "bigint",
            F::Integer,
            Typed::Text,
            vec![
                ("9223372036854775807", int(i64::MAX)),
                ("-9223372036854775808", int(i64::MIN)),
                ("9007199254740993", int(9_007_199_254_740_993)),
                ("-9007199254740993", int(-9_007_199_254_740_993)),
            ],
        ),
        case(
            "oid",
            F::Integer,
            Typed::Text,
            vec![("4294967295", int(4_294_967_295))],
        ),
        case(
            "numeric(40,20)",
            F::Decimal,
            Typed::Text,
            vec![
                (
                    "12345678901234567890.12345678901234567890",
                    dec("12345678901234567890.12345678901234567890"),
                ),
                ("0.00000000000000000001", dec("0.00000000000000000001")),
                ("-0.5", dec("-0.50000000000000000000")),
                ("0", dec("0.00000000000000000000")),
            ],
        ),
        case(
            "numeric",
            F::Decimal,
            Typed::Text,
            vec![
                ("'NaN'", dec("NaN")),
                (
                    "123456789012345678901234567890.123456789",
                    dec("123456789012345678901234567890.123456789"),
                ),
                ("1e100", dec(&format!("1{}", "0".repeat(100)))),
                (
                    "-0.000000000000000000000000000001",
                    dec("-0.000000000000000000000000000001"),
                ),
                ("100.00", dec("100.00")),
            ],
        ),
        case(
            "real",
            F::Float,
            Typed::Text,
            vec![
                ("1.5", float(1.5)),
                ("0.1", float(0.1)),
                ("'NaN'", float(f64::NAN)),
                ("'Infinity'", float(f64::INFINITY)),
                ("'-Infinity'", float(f64::NEG_INFINITY)),
                ("3.4e38", float(3.4e38)),
                ("-1.5e-10", float(-1.5e-10)),
            ],
        ),
        case(
            "double precision",
            F::Float,
            Typed::Text,
            vec![
                ("0.30000000000000004", float(0.30000000000000004)),
                ("1.7976931348623157e308", float(f64::MAX)),
                ("5e-324", float(5e-324)),
                ("'NaN'", float(f64::NAN)),
                ("'Infinity'", float(f64::INFINITY)),
                ("'-Infinity'", float(f64::NEG_INFINITY)),
                ("0", float(0.0)),
                ("-1234567.891", float(-1234567.891)),
            ],
        ),
        case(
            "boolean",
            F::Boolean,
            Typed::Text,
            vec![("true", Value::Bool(true)), ("false", Value::Bool(false))],
        ),
        case(
            "text",
            F::Text,
            Typed::Text,
            vec![
                ("''", text("")),
                ("' '", text(" ")),
                ("'  padded  '", text("  padded  ")),
                ("'plain'", text("plain")),
                (
                    "'it''s \"quoted\" back\\slash'",
                    text("it's \"quoted\" back\\slash"),
                ),
                ("E'tab\\there'", text("tab\there")),
                ("E'line1\\nline2\\r\\nline3'", text("line1\nline2\r\nline3")),
                ("E'\\x01\\x1f\\x7f'", text("\u{1}\u{1f}\u{7f}")),
                (&format!("'{}'", long_unicode()), text(&long_unicode())),
                ("'日本語'", text("日本語")),
                ("'🎉'", text("🎉")),
                ("'NULL'", text("NULL")),
                ("'null'", text("null")),
                ("'\\x'", text("\\x")),
                ("E'\\\\'", text("\\")),
            ],
        ),
        case(
            "varchar(5)",
            F::Text,
            Typed::Text,
            vec![("'abc'", text("abc")), ("''", text(""))],
        ),
        case(
            "char(5)",
            F::Text,
            Typed::Text,
            vec![("'ab'", text("ab   ")), ("'abcde'", text("abcde"))],
        ),
        case(
            "name",
            F::Text,
            Typed::Text,
            vec![("'pg_class'", text("pg_class"))],
        ),
        case("\"char\"", F::Text, Typed::Text, vec![("'a'", text("a"))]),
        case(
            "bytea",
            F::Binary,
            Typed::Bytes,
            vec![
                ("'\\xdeadbeef'", bytes(&[0xde, 0xad, 0xbe, 0xef])),
                ("'\\x'", bytes(&[])),
                ("'\\x00'", bytes(&[0])),
                ("'\\x00ff00ff'", bytes(&[0, 0xff, 0, 0xff])),
                ("decode('ff', 'hex')", bytes(&[0xff])),
                ("'plain text'::bytea", bytes(b"plain text")),
                ("E'\\\\000\\\\001'::bytea", bytes(&[0, 1])),
            ],
        ),
        case(
            "json",
            F::Json,
            Typed::Json,
            vec![
                (
                    "'{\"a\":[1,2,{\"b\":null}],\"c\":\"d\"}'",
                    json(serde_json::json!({"a": [1, 2, {"b": null}], "c": "d"})),
                ),
                ("'null'", json(serde_json::Value::Null)),
                (
                    "'\"just a string\"'",
                    json(serde_json::json!("just a string")),
                ),
                ("'42'", json(serde_json::json!(42))),
                ("'3.5'", json(serde_json::json!(3.5))),
                ("'true'", json(serde_json::json!(true))),
                ("'[]'", json(serde_json::json!([]))),
                ("'{}'", json(serde_json::json!({}))),
                (
                    "'{\"ünï\":\"🎉\",\"\":1}'",
                    json(serde_json::json!({"ünï": "🎉", "": 1})),
                ),
                (
                    "'[1,[2,[3,[4,[5,[6,[7,[8,[9,[10]]]]]]]]]]'",
                    json(serde_json::json!([
                        1,
                        [2, [3, [4, [5, [6, [7, [8, [9, [10]]]]]]]]]
                    ])),
                ),
                (
                    "'{\"s\":\"quote\\\" back\\\\ nl\\n tab\\t \\u00e9\"}'",
                    json(serde_json::json!({"s": "quote\" back\\ nl\n tab\t é"})),
                ),
            ],
        ),
        case(
            "jsonb",
            F::Json,
            Typed::Json,
            vec![
                (
                    "'{\"b\":1,\"a\":[1,2,{\"c\":null}]}'",
                    json(serde_json::json!({"a": [1, 2, {"c": null}], "b": 1})),
                ),
                ("'null'", json(serde_json::Value::Null)),
                ("'\"x\"'", json(serde_json::json!("x"))),
                ("'123456789'", json(serde_json::json!(123456789))),
                (
                    "'{\"big\":9007199254740993}'",
                    json(serde_json::json!({"big": 9_007_199_254_740_993_i64})),
                ),
                ("'{\"a\":1,\"a\":2}'", json(serde_json::json!({"a": 2}))),
                (
                    "'[null,true,false,0,\"\"]'",
                    json(serde_json::json!([null, true, false, 0, ""])),
                ),
            ],
        ),
        case(
            "uuid",
            F::Uuid,
            Typed::Text,
            vec![
                (
                    "'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'",
                    Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
                ),
                (
                    "'A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11'",
                    Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
                ),
                (
                    "'00000000-0000-0000-0000-000000000000'",
                    Value::Uuid("00000000-0000-0000-0000-000000000000".into()),
                ),
                (
                    "'{a0eebc999c0b4ef8bb6d6bb9bd380a11}'",
                    Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
                ),
            ],
        ),
        case(
            "date",
            F::Date,
            Typed::Text,
            vec![
                ("'2024-02-29'", date("2024-02-29")),
                ("'1900-01-01'", date("1900-01-01")),
                ("'1969-12-31'", date("1969-12-31")),
                ("'1970-01-01'", date("1970-01-01")),
                ("'0001-01-01'", date("0001-01-01")),
                ("'0044-03-15 BC'", date("0044-03-15 BC")),
                ("'9999-12-31'", date("9999-12-31")),
                ("'5874897-12-31'", date("5874897-12-31")),
                ("'infinity'", date("infinity")),
                ("'-infinity'", date("-infinity")),
            ],
        ),
        case(
            "time",
            F::Time,
            Typed::Text,
            vec![
                ("'00:00:00'", time("00:00:00")),
                ("'23:59:59.999999'", time("23:59:59.999999")),
                ("'12:34:56.5'", time("12:34:56.5")),
                ("'24:00:00'", time("24:00:00")),
            ],
        ),
        case(
            "time with time zone",
            F::Time,
            Typed::Text,
            vec![
                ("'12:34:56+05:30'", time("12:34:56+05:30")),
                ("'00:00:00-12'", time("00:00:00-12")),
            ],
        ),
        case(
            "timestamp",
            F::DateTime,
            Typed::Text,
            vec![
                (
                    "'2024-02-29 13:14:15.123456'",
                    datetime("2024-02-29 13:14:15.123456"),
                ),
                ("'1969-12-31 23:59:59'", datetime("1969-12-31 23:59:59")),
                ("'1900-01-01 00:00:00'", datetime("1900-01-01 00:00:00")),
                ("'0001-01-01 00:00:00'", datetime("0001-01-01 00:00:00")),
                (
                    "'0044-03-15 12:00:00 BC'",
                    datetime("0044-03-15 12:00:00 BC"),
                ),
                (
                    "'9999-12-31 23:59:59.999999'",
                    datetime("9999-12-31 23:59:59.999999"),
                ),
                ("'294276-12-31 23:59:59'", datetime("294276-12-31 23:59:59")),
                ("'infinity'", datetime("infinity")),
                ("'-infinity'", datetime("-infinity")),
                ("'2024-02-29 13:14:15+09'", datetime("2024-02-29 13:14:15")),
            ],
        ),
        case(
            "timestamp with time zone",
            F::DateTime,
            Typed::Text,
            vec![
                (
                    "'2024-07-01 12:00:00+00'",
                    datetime("2024-07-01 12:00:00+00"),
                ),
                (
                    "'2024-07-01 12:00:00+02'",
                    datetime("2024-07-01 10:00:00+00"),
                ),
                (
                    "'2024-01-15 12:00:00.123456-05'",
                    datetime("2024-01-15 17:00:00.123456+00"),
                ),
                (
                    "'1900-01-01 00:00:00+00'",
                    datetime("1900-01-01 00:00:00+00"),
                ),
                (
                    "'9999-12-31 23:59:59+00'",
                    datetime("9999-12-31 23:59:59+00"),
                ),
                ("'infinity'", datetime("infinity")),
                ("'-infinity'", datetime("-infinity")),
            ],
        ),
        case(
            "interval",
            F::Other,
            Typed::Text,
            vec![
                (
                    "'1 year 2 mons 3 days 04:05:06.789'",
                    other("interval", "1 year 2 mons 3 days 04:05:06.789"),
                ),
                ("'-1 days'", other("interval", "-1 days")),
                ("'00:00:00'", other("interval", "00:00:00")),
                ("'infinity'", other("interval", "infinity")),
            ],
        ),
        case(
            "money",
            F::Decimal,
            Typed::Text,
            vec![("'1234.56'", dec("$1,234.56")), ("'-0.01'", dec("-$0.01"))],
        ),
        case(
            "bit(8)",
            F::Other,
            Typed::Text,
            vec![
                ("B'10101010'", other("bit", "10101010")),
                ("B'00000000'", other("bit", "00000000")),
            ],
        ),
        case(
            "bit varying(10)",
            F::Other,
            Typed::Text,
            vec![("B'101'", other("varbit", "101"))],
        ),
        case(
            "inet",
            F::Other,
            Typed::Text,
            vec![
                ("'192.168.0.1/24'", other("inet", "192.168.0.1/24")),
                ("'::1'", other("inet", "::1")),
                ("'10.0.0.1'", other("inet", "10.0.0.1")),
            ],
        ),
        case(
            "cidr",
            F::Other,
            Typed::Text,
            vec![("'10.0.0.0/8'", other("cidr", "10.0.0.0/8"))],
        ),
        case(
            "macaddr",
            F::Other,
            Typed::Text,
            vec![("'08:00:2b:01:02:03'", other("macaddr", "08:00:2b:01:02:03"))],
        ),
        case(
            "point",
            F::Spatial,
            Typed::Text,
            vec![
                ("'(1,2)'", other("point", "(1,2)")),
                ("'(-1.5,2.25)'", other("point", "(-1.5,2.25)")),
            ],
        ),
        case(
            "box",
            F::Spatial,
            Typed::Text,
            vec![("'((0,0),(1,1))'", other("box", "(1,1),(0,0)"))],
        ),
        case(
            "circle",
            F::Spatial,
            Typed::Text,
            vec![("'<(1,2),3>'", other("circle", "<(1,2),3>"))],
        ),
        case(
            "xml",
            F::Text,
            Typed::Text,
            vec![("'<a>b&amp;c</a>'", text("<a>b&amp;c</a>"))],
        ),
        case(
            "tsvector",
            F::Other,
            Typed::Text,
            vec![(
                "to_tsvector('simple', 'hello world')",
                other("tsvector", "'hello':1 'world':2"),
            )],
        ),
        case(
            "int4range",
            F::Other,
            Typed::Text,
            vec![
                ("'[1,10)'", other("int4range", "[1,10)")),
                ("'empty'", other("int4range", "empty")),
            ],
        ),
        case(
            "daterange",
            F::Other,
            Typed::Text,
            vec![(
                "'[2024-01-01,2024-02-01)'",
                other("daterange", "[2024-01-01,2024-02-01)"),
            )],
        ),
        case(
            "tstzrange",
            F::Other,
            Typed::No,
            vec![(
                "'[2024-01-01 00:00:00+00,2024-02-01 00:00:00+00)'",
                other(
                    "tstzrange",
                    "[\"2024-01-01 00:00:00+00\",\"2024-02-01 00:00:00+00\")",
                ),
            )],
        ),
        case(
            "int8multirange",
            F::Other,
            Typed::Text,
            vec![(
                "'{[1,5),[10,20)}'",
                other("int8multirange", "{[1,5),[10,20)}"),
            )],
        ),
        case(
            "citext",
            F::Text,
            Typed::Text,
            vec![("'MiXeD'", text("MiXeD"))],
        ),
        case(
            "hstore",
            F::Other,
            Typed::Text,
            vec![(
                "'a=>1, b=>NULL, \"c d\"=>\"e,f\"'",
                other("hstore", "\"a\"=>\"1\", \"b\"=>NULL, \"c d\"=>\"e,f\""),
            )],
        ),
    ];
    run_cases(c, cases).await;
}
each_engine!(pg_scalar_types, postgres);

async fn pg_array_enum_domain_and_composite_types(c: &Ctx) {
    use TypeFamily as F;
    let mood = c.name("mood");
    let dom = c.name("posint");
    let comp = c.name("addr");
    c.cleanup_sql(format!("DROP TYPE IF EXISTS {} CASCADE", c.q(&mood)));
    c.cleanup_sql(format!("DROP DOMAIN IF EXISTS {} CASCADE", c.q(&dom)));
    c.cleanup_sql(format!("DROP TYPE IF EXISTS {} CASCADE", c.q(&comp)));
    c.side_exec(&format!(
        "CREATE TYPE {} AS ENUM ('sad', 'ok', 'happy', 'ünïcødé 🎉', 'with space', 'it''s')",
        c.q(&mood)
    ))
    .await;
    c.side_exec(&format!(
        "CREATE DOMAIN {} AS integer CHECK (VALUE > 0)",
        c.q(&dom)
    ))
    .await;
    c.side_exec(&format!(
        "CREATE TYPE {} AS (street text, city text, zip varchar(10))",
        c.q(&comp)
    ))
    .await;
    let mood_q = c.q(&mood);
    let cases = vec![
        case(
            "integer[]",
            F::Array,
            Typed::Text,
            vec![
                ("'{1,2,3}'", other("int4[]", "{1,2,3}")),
                ("'{}'", other("int4[]", "{}")),
                ("'{1,NULL,3}'", other("int4[]", "{1,NULL,3}")),
                ("'{{1,2},{3,4}}'", other("int4[]", "{{1,2},{3,4}}")),
            ],
        ),
        case(
            "bigint[]",
            F::Array,
            Typed::Text,
            vec![(
                "'{9223372036854775807}'",
                other("int8[]", "{9223372036854775807}"),
            )],
        ),
        case(
            "text[]",
            F::Array,
            Typed::Text,
            vec![
                (
                    "ARRAY['a b','c,d',NULL,'e\"f','g\\h','NULL','']",
                    other(
                        "text[]",
                        "{\"a b\",\"c,d\",NULL,\"e\\\"f\",\"g\\\\h\",\"NULL\",\"\"}",
                    ),
                ),
                ("ARRAY['🎉','日本']", other("text[]", "{🎉,日本}")),
            ],
        ),
        case(
            "numeric(10,2)[]",
            F::Array,
            Typed::Text,
            vec![("'{1.50,2.25}'", other("numeric[]", "{1.50,2.25}"))],
        ),
        case(
            "boolean[]",
            F::Array,
            Typed::Text,
            vec![("'{t,f}'", other("bool[]", "{t,f}"))],
        ),
        case(
            "uuid[]",
            F::Array,
            Typed::Text,
            vec![(
                "'{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}'",
                other("uuid[]", "{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}"),
            )],
        ),
        case(
            "timestamptz[]",
            F::Array,
            Typed::Text,
            vec![(
                "'{\"2024-01-01 00:00:00+00\"}'",
                other("timestamptz[]", "{\"2024-01-01 00:00:00+00\"}"),
            )],
        ),
        case(
            "jsonb[]",
            F::Array,
            Typed::No,
            vec![(
                "ARRAY['{\"a\":1}'::jsonb]",
                other("jsonb[]", "{\"{\\\"a\\\": 1}\"}"),
            )],
        ),
        case(
            "bytea[]",
            F::Array,
            Typed::Text,
            vec![(
                "ARRAY['\\xdead'::bytea]",
                other("bytea[]", "{\"\\\\xdead\"}"),
            )],
        ),
        case(
            &mood_q,
            F::Enum,
            Typed::Text,
            vec![
                ("'happy'", text("happy")),
                ("'ünïcødé 🎉'", text("ünïcødé 🎉")),
                ("'with space'", text("with space")),
                ("'it''s'", text("it's")),
            ],
        ),
        case(
            &format!("{mood_q}[]"),
            F::Array,
            Typed::Text,
            vec![(
                &format!("ARRAY['happy','sad']::{mood_q}[]"),
                other(&format!("{mood}[]"), "{happy,sad}"),
            )],
        ),
        case(
            &c.q(&dom),
            F::Integer,
            Typed::Text,
            vec![("7", int(7)), ("2147483647", int(i32::MAX as i64))],
        ),
        case(
            &c.q(&comp),
            F::Other,
            Typed::Text,
            vec![
                (
                    "ROW('1 Main St', 'Springfield', '12345')",
                    other(&comp, "(\"1 Main St\",Springfield,12345)"),
                ),
                (
                    "ROW('say \"hi\"', NULL, '')",
                    other(&comp, "(\"say \"\"hi\"\"\",,\"\")"),
                ),
            ],
        ),
    ];
    run_cases(c, cases).await;
}
each_engine!(pg_array_enum_domain_and_composite_types, postgres);

// ---------------------------------------------------------------------------
// MySQL and MariaDB
// ---------------------------------------------------------------------------

fn nul_text() -> Value {
    text("a\u{0}b")
}

async fn my_scalar_types(c: &Ctx) {
    use TypeFamily as F;
    // Zero dates and other legacy values need a permissive session on the side channel.
    c.side_exec("SET SESSION sql_mode = ''").await;
    let mut cases = vec![
        case(
            "tinyint(1)",
            F::Boolean,
            Typed::BoolNum,
            vec![
                ("0", Value::Bool(false)),
                ("1", Value::Bool(true)),
                ("2", int(2)),
                ("-1", int(-1)),
            ],
        ),
        case(
            "tinyint",
            F::Integer,
            Typed::Text,
            vec![("127", int(127)), ("-128", int(-128))],
        ),
        case(
            "tinyint unsigned",
            F::Integer,
            Typed::Text,
            vec![("255", int(255)), ("0", int(0))],
        ),
        case(
            "smallint",
            F::Integer,
            Typed::Text,
            vec![("32767", int(32767)), ("-32768", int(-32768))],
        ),
        case(
            "smallint unsigned",
            F::Integer,
            Typed::Text,
            vec![("65535", int(65535))],
        ),
        case(
            "mediumint",
            F::Integer,
            Typed::Text,
            vec![("8388607", int(8_388_607)), ("-8388608", int(-8_388_608))],
        ),
        case(
            "mediumint unsigned",
            F::Integer,
            Typed::Text,
            vec![("16777215", int(16_777_215))],
        ),
        case(
            "int",
            F::Integer,
            Typed::Text,
            vec![
                ("2147483647", int(i32::MAX as i64)),
                ("-2147483648", int(i32::MIN as i64)),
            ],
        ),
        case(
            "int unsigned",
            F::Integer,
            Typed::Text,
            vec![("4294967295", int(u32::MAX as i64))],
        ),
        case(
            "bigint",
            F::Integer,
            Typed::Text,
            vec![
                ("9223372036854775807", int(i64::MAX)),
                ("-9223372036854775808", int(i64::MIN)),
                ("9007199254740993", int(9_007_199_254_740_993)),
            ],
        ),
        case(
            "bigint unsigned",
            F::Integer,
            Typed::Text,
            vec![
                ("18446744073709551615", Value::UInt(u64::MAX)),
                (
                    "9223372036854775808",
                    Value::UInt(9_223_372_036_854_775_808),
                ),
                ("9223372036854775807", int(i64::MAX)),
                ("0", int(0)),
            ],
        ),
        case(
            "year",
            F::Integer,
            Typed::Text,
            vec![
                ("2024", int(2024)),
                ("1901", int(1901)),
                ("2155", int(2155)),
            ],
        ),
        case(
            "decimal(65,30)",
            F::Decimal,
            Typed::Text,
            vec![
                (
                    "12345678901234567890123456789012345.123456789012345678901234567890",
                    dec("12345678901234567890123456789012345.123456789012345678901234567890"),
                ),
                (
                    "-0.000000000000000000000000000001",
                    dec("-0.000000000000000000000000000001"),
                ),
                ("0", dec("0.000000000000000000000000000000")),
            ],
        ),
        case(
            "decimal(10,2)",
            F::Decimal,
            Typed::Text,
            vec![
                ("-0.5", dec("-0.50")),
                ("99999999.99", dec("99999999.99")),
                ("100", dec("100.00")),
            ],
        ),
        case(
            "decimal(20,0)",
            F::Decimal,
            Typed::Text,
            vec![("12345678901234567890", dec("12345678901234567890"))],
        ),
        case(
            "float",
            F::Float,
            Typed::Text,
            vec![
                ("1.5", float(1.5)),
                ("0.1", float(0.1)),
                ("-3.5e10", float(-3.5e10)),
                ("0", float(0.0)),
            ],
        ),
        case(
            "double",
            F::Float,
            Typed::Text,
            vec![
                ("0.30000000000000004", float(0.30000000000000004)),
                ("1.7976931348623157e308", float(f64::MAX)),
                ("-1234567.891", float(-1234567.891)),
                ("1e-300", float(1e-300)),
                ("0", float(0.0)),
            ],
        ),
        case(
            "bit(8)",
            F::Other,
            Typed::No,
            vec![
                ("b'10101010'", other("BIT", "b'10101010'")),
                ("b'1'", other("BIT", "b'1'")),
                ("b'0'", other("BIT", "b'0'")),
            ],
        ),
        case(
            "bit(64)",
            F::Other,
            Typed::No,
            vec![(
                &format!("b'{}'", "1".repeat(64)),
                other("BIT", &format!("b'{}'", "1".repeat(64))),
            )],
        ),
        case(
            "char(5)",
            F::Text,
            Typed::Text,
            vec![
                ("'ab'", text("ab")),
                ("'abcde'", text("abcde")),
                ("''", text("")),
            ],
        ),
        case(
            "varchar(100)",
            F::Text,
            Typed::Text,
            vec![
                ("''", text("")),
                ("'  padded  '", text("  padded  ")),
                (
                    "'it''s \"quoted\" back\\\\slash'",
                    text("it's \"quoted\" back\\slash"),
                ),
                (
                    "'tab\\there\\nnewline\\rcr'",
                    text("tab\there\nnewline\rcr"),
                ),
                ("'a\\0b'", nul_text()),
                (&format!("'{}'", long_unicode()), text(&long_unicode())),
                ("'日本語'", text("日本語")),
                ("'🎉 party'", text("🎉 party")),
                ("'NULL'", text("NULL")),
                ("'\\\\'", text("\\")),
            ],
        ),
        case("tinytext", F::Text, Typed::Text, vec![("'t'", text("t"))]),
        case(
            "text",
            F::Text,
            Typed::Text,
            vec![("'hello'", text("hello")), ("''", text(""))],
        ),
        case("mediumtext", F::Text, Typed::Text, vec![("'m'", text("m"))]),
        case("longtext", F::Text, Typed::Text, vec![("'l'", text("l"))]),
        // `_bin` collations are text, not bytes.
        case(
            "varchar(10) COLLATE utf8mb4_bin",
            F::Text,
            Typed::Text,
            vec![("'MiXeD'", text("MiXeD"))],
        ),
        case(
            "char(5) BINARY",
            F::Text,
            Typed::Text,
            vec![("'ab'", text("ab"))],
        ),
        case(
            "text COLLATE utf8mb4_bin",
            F::Text,
            Typed::Text,
            vec![("'bin'", text("bin"))],
        ),
        case(
            "varchar(10) CHARACTER SET latin1",
            F::Text,
            Typed::Text,
            vec![("'é'", text("é")), ("'abc'", text("abc"))],
        ),
        case(
            "binary(4)",
            F::Binary,
            Typed::Bytes,
            vec![
                ("'ab'", bytes(&[0x61, 0x62, 0, 0])),
                ("X'DEADBEEF'", bytes(&[0xde, 0xad, 0xbe, 0xef])),
            ],
        ),
        case(
            "varbinary(20)",
            F::Binary,
            Typed::Bytes,
            vec![
                ("X'DEADBEEF'", bytes(&[0xde, 0xad, 0xbe, 0xef])),
                ("X''", bytes(&[])),
                ("X'00'", bytes(&[0])),
                ("X'00FF00'", bytes(&[0, 0xff, 0])),
            ],
        ),
        case(
            "tinyblob",
            F::Binary,
            Typed::Bytes,
            vec![("X'01'", bytes(&[1]))],
        ),
        case(
            "blob",
            F::Binary,
            Typed::Bytes,
            vec![("X'00FF'", bytes(&[0, 0xff])), ("'plain'", bytes(b"plain"))],
        ),
        case(
            "mediumblob",
            F::Binary,
            Typed::Bytes,
            vec![("X'02'", bytes(&[2]))],
        ),
        case(
            "longblob",
            F::Binary,
            Typed::Bytes,
            vec![("X'03'", bytes(&[3]))],
        ),
        case(
            "enum('a','b','it''s','ünï')",
            F::Enum,
            Typed::Text,
            vec![
                ("'a'", text("a")),
                ("'it''s'", text("it's")),
                ("'ünï'", text("ünï")),
            ],
        ),
        case(
            "set('x','y','z')",
            F::Enum,
            Typed::Text,
            vec![
                ("'x,z'", text("x,z")),
                ("''", text("")),
                ("'x,y,z'", text("x,y,z")),
            ],
        ),
        case(
            "date",
            F::Date,
            Typed::Text,
            vec![
                ("'2024-02-29'", date("2024-02-29")),
                ("'1000-01-01'", date("1000-01-01")),
                ("'9999-12-31'", date("9999-12-31")),
                ("'0001-01-01'", date("0001-01-01")),
                ("'0000-00-00'", date("0000-00-00")),
                ("'2024-00-00'", date("2024-00-00")),
            ],
        ),
        case(
            "datetime",
            F::DateTime,
            Typed::Text,
            vec![
                ("'2024-02-29 13:14:15'", datetime("2024-02-29 13:14:15")),
                ("'1000-01-01 00:00:00'", datetime("1000-01-01 00:00:00")),
                ("'9999-12-31 23:59:59'", datetime("9999-12-31 23:59:59")),
                ("'0000-00-00 00:00:00'", datetime("0000-00-00 00:00:00")),
            ],
        ),
        case(
            "datetime(6)",
            F::DateTime,
            Typed::Text,
            vec![
                (
                    "'2024-02-29 13:14:15.123456'",
                    datetime("2024-02-29 13:14:15.123456"),
                ),
                (
                    "'2024-02-29 13:14:15.1'",
                    datetime("2024-02-29 13:14:15.100000"),
                ),
            ],
        ),
        case(
            "datetime(3)",
            F::DateTime,
            Typed::Text,
            vec![(
                "'2024-02-29 13:14:15.123456'",
                datetime("2024-02-29 13:14:15.123"),
            )],
        ),
        case(
            "timestamp NULL",
            F::DateTime,
            Typed::Text,
            vec![
                ("'2024-07-01 12:00:00'", datetime("2024-07-01 12:00:00")),
                ("'1970-01-01 00:00:01'", datetime("1970-01-01 00:00:01")),
                ("'2038-01-19 03:14:07'", datetime("2038-01-19 03:14:07")),
            ],
        ),
        case(
            "timestamp(6) NULL",
            F::DateTime,
            Typed::Text,
            vec![(
                "'2024-07-01 12:00:00.654321'",
                datetime("2024-07-01 12:00:00.654321"),
            )],
        ),
        case(
            "time",
            F::Time,
            Typed::Text,
            vec![
                ("'00:00:00'", time("00:00:00")),
                ("'838:59:59'", time("838:59:59")),
                ("'-838:59:59'", time("-838:59:59")),
                ("'12:34:56'", time("12:34:56")),
            ],
        ),
        case(
            "time(6)",
            F::Time,
            Typed::Text,
            vec![
                ("'12:34:56.123456'", time("12:34:56.123456")),
                ("'-00:00:01.5'", time("-00:00:01.500000")),
            ],
        ),
        case(
            "point",
            F::Spatial,
            Typed::Bytes,
            vec![(
                "ST_GeomFromText('POINT(1 2)')",
                bytes(&[
                    0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0, 0, 0,
                    0x40,
                ]),
            )],
        ),
    ];
    if c.is_mysql() {
        cases.push(case(
            "json",
            F::Json,
            Typed::Json,
            vec![
                (
                    "'{\"b\":1,\"a\":[1,2,{\"c\":null}]}'",
                    json(serde_json::json!({"a": [1, 2, {"c": null}], "b": 1})),
                ),
                ("'null'", json(serde_json::Value::Null)),
                ("'\"str\"'", json(serde_json::json!("str"))),
                ("'[]'", json(serde_json::json!([]))),
                ("'{}'", json(serde_json::json!({}))),
                ("'1.5'", json(serde_json::json!(1.5))),
                ("'true'", json(serde_json::json!(true))),
                (
                    "'{\"big\":9007199254740993}'",
                    json(serde_json::json!({"big": 9_007_199_254_740_993_i64})),
                ),
                ("'{\"ünï\":\"🎉\"}'", json(serde_json::json!({"ünï": "🎉"}))),
            ],
        ));
    } else {
        // MariaDB's JSON is an alias for LONGTEXT: the text is kept as typed.
        cases.push(case(
            "json",
            F::Text,
            Typed::Text,
            vec![
                ("'{\"b\":1,\"a\":[1,2]}'", text("{\"b\":1,\"a\":[1,2]}")),
                ("'null'", text("null")),
            ],
        ));
        // The wire protocol reports MariaDB's UUID type as CHAR, so it shows as text.
        let mut uuid = case(
            "uuid",
            F::Text,
            Typed::Text,
            vec![(
                "'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'",
                text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
            )],
        );
        uuid.family = None;
        cases.push(uuid);
    }
    run_cases(c, cases).await;
}
each_engine!(my_scalar_types, mysql, mariadb);

// ---------------------------------------------------------------------------
// SQLite: values keep their storage class
// ---------------------------------------------------------------------------

async fn sqlite_storage_classes(c: &Ctx) {
    use TypeFamily as F;
    let c_case = |ty: &str, family: Option<TypeFamily>, typed, rows: Vec<(&str, Value)>| {
        let mut k = case(ty, F::Other, typed, rows);
        k.family = family;
        k
    };
    let cases = vec![
        c_case(
            "INTEGER",
            Some(F::Integer),
            Typed::Text,
            vec![
                ("9223372036854775807", int(i64::MAX)),
                ("-9223372036854775808", int(i64::MIN)),
                ("9007199254740993", int(9_007_199_254_740_993)),
                ("0", int(0)),
                ("'12'", int(12)),
                ("'abc'", text("abc")),
                ("1.5", float(1.5)),
            ],
        ),
        c_case(
            "BIGINT",
            Some(F::Integer),
            Typed::Text,
            vec![("42", int(42))],
        ),
        c_case(
            "BOOLEAN",
            Some(F::Boolean),
            Typed::BoolNum,
            vec![
                ("0", Value::Bool(false)),
                ("1", Value::Bool(true)),
                ("2", int(2)),
                ("'true'", text("true")),
                ("-1", int(-1)),
            ],
        ),
        c_case(
            "REAL",
            Some(F::Float),
            Typed::Text,
            vec![
                ("1.5", float(1.5)),
                ("0.1", float(0.1)),
                ("0.30000000000000004", float(0.30000000000000004)),
                ("1e308", float(1e308)),
                ("-1e308", float(-1e308)),
                ("5e-324", float(5e-324)),
                ("1", float(1.0)),
                ("'7.25'", float(7.25)),
            ],
        ),
        c_case(
            "REAL",
            Some(F::Float),
            Typed::No,
            vec![
                ("9e999", float(f64::INFINITY)),
                ("-9e999", float(f64::NEG_INFINITY)),
            ],
        ),
        c_case(
            "DOUBLE PRECISION",
            Some(F::Float),
            Typed::Text,
            vec![("2.5", float(2.5))],
        ),
        c_case(
            "TEXT",
            Some(F::Text),
            Typed::Text,
            vec![
                ("''", text("")),
                ("'  padded  '", text("  padded  ")),
                (
                    "'it''s \"quoted\" back\\slash'",
                    text("it's \"quoted\" back\\slash"),
                ),
                ("'tab\there'", text("tab\there")),
                (
                    "'line1' || char(10) || 'line2' || char(13)",
                    text("line1\nline2\r"),
                ),
                ("char(97, 0, 98)", nul_text()),
                (&format!("'{}'", long_unicode()), text(&long_unicode())),
                ("'日本語 🎉'", text("日本語 🎉")),
                ("'NULL'", text("NULL")),
                ("12345", text("12345")),
                ("1.5", text("1.5")),
            ],
        ),
        c_case(
            "VARCHAR(5)",
            Some(F::Text),
            Typed::Text,
            vec![(
                "'longer than five characters'",
                text("longer than five characters"),
            )],
        ),
        c_case(
            "BLOB",
            Some(F::Binary),
            Typed::Bytes,
            vec![
                ("X'DEADBEEF'", bytes(&[0xde, 0xad, 0xbe, 0xef])),
                ("X''", bytes(&[])),
                ("X'00'", bytes(&[0])),
                ("X'00FF00'", bytes(&[0, 0xff, 0])),
                ("zeroblob(3)", bytes(&[0, 0, 0])),
                ("CAST('abc' AS BLOB)", bytes(b"abc")),
            ],
        ),
        // NUMERIC affinity keeps integers as integers and converts other numbers to REAL.
        // The family of NUMERIC columns follows the first row's storage class.
        c_case(
            "NUMERIC",
            None,
            Typed::Text,
            vec![
                ("100.00", int(100)),
                ("12.5", float(12.5)),
                ("'42'", int(42)),
                ("'n/a'", text("n/a")),
            ],
        ),
        c_case(
            "DECIMAL(10,2)",
            None,
            Typed::Text,
            vec![("12.25", float(12.25)), ("7", int(7))],
        ),
        // Dates are text (or numbers) as stored.
        c_case(
            "DATE",
            Some(F::Date),
            Typed::Text,
            vec![
                ("'2024-02-29'", text("2024-02-29")),
                ("'0001-01-01'", text("0001-01-01")),
                ("'9999-12-31'", text("9999-12-31")),
                ("'1969-12-31'", text("1969-12-31")),
                ("'0000-00-00'", text("0000-00-00")),
            ],
        ),
        c_case(
            "DATETIME",
            Some(F::DateTime),
            Typed::Text,
            vec![
                ("'2024-02-29 13:14:15.123'", text("2024-02-29 13:14:15.123")),
                (
                    "'2024-07-01T12:00:00+05:30'",
                    text("2024-07-01T12:00:00+05:30"),
                ),
                ("'1900-01-01 00:00:00'", text("1900-01-01 00:00:00")),
                ("1700000000", int(1_700_000_000)),
            ],
        ),
        c_case(
            "TIMESTAMP",
            Some(F::DateTime),
            Typed::Text,
            vec![("'2024-07-01 12:00:00'", text("2024-07-01 12:00:00"))],
        ),
        c_case(
            "TIME",
            Some(F::Time),
            Typed::Text,
            vec![("'12:34:56'", text("12:34:56"))],
        ),
        // Declared types SQLite has no storage for are kept as text.
        c_case(
            "JSON",
            None,
            Typed::Text,
            vec![
                ("'{\"a\":[1,2]}'", text("{\"a\":[1,2]}")),
                ("'null'", text("null")),
            ],
        ),
        c_case(
            "UUID",
            None,
            Typed::Text,
            vec![(
                "'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'",
                text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
            )],
        ),
        c_case(
            "ANY",
            None,
            Typed::Text,
            vec![("1", int(1)), ("'x'", text("x")), ("1.5", float(1.5))],
        ),
        c_case("ANY", None, Typed::Bytes, vec![("X'AB'", bytes(&[0xab]))]),
    ];
    run_cases(c, cases).await;
}
each_engine!(sqlite_storage_classes, sqlite);

async fn sqlite_expression_columns_take_their_family_from_the_value(c: &Ctx) {
    use TypeFamily as F;
    let events = c
        .run("SELECT 1 + 1 AS i, 'a' || 'b' AS t, x'00ff' AS b, NULL AS n, 1.5 * 2 AS r, 9223372036854775807 AS big")
        .await;
    let set = &sets_of(&events)[0];
    let fam: Vec<TypeFamily> = set.columns.iter().map(|c| c.family).collect();
    assert_eq!(
        fam,
        [
            F::Integer,
            F::Text,
            F::Binary,
            F::Other,
            F::Float,
            F::Integer
        ]
    );
    assert_eq!(
        set.rows[0],
        vec![
            int(2),
            text("ab"),
            bytes(&[0, 0xff]),
            Value::Null,
            float(3.0),
            int(i64::MAX)
        ]
    );
    // The same column can hold several storage classes.
    let t = c.scratch("mixed");
    c.side_exec(&format!(
        "CREATE TABLE {} (id INTEGER PRIMARY KEY, v)",
        c.qt(&t)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, 1), (2, 'two'), (3, 3.5), (4, x'04'), (5, NULL)",
        c.qt(&t)
    ))
    .await;
    let rows = c.page(&c.browse(&t), 0, 10).await.rows;
    let vals: Vec<Value> = rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(
        vals,
        vec![int(1), text("two"), float(3.5), bytes(&[4]), Value::Null]
    );
}
each_engine!(
    sqlite_expression_columns_take_their_family_from_the_value,
    sqlite
);

// ---------------------------------------------------------------------------
// NULL versus empty, long values, time zones
// ---------------------------------------------------------------------------

async fn null_is_not_empty_text(c: &Ctx) {
    let t = c.scratch("nulls");
    let text_ty = match c.engine {
        Engine::Postgres => "text",
        Engine::Sqlite => "TEXT",
        _ => "VARCHAR(20)",
    };
    c.side_exec(&format!(
        "CREATE TABLE {} (id {} PRIMARY KEY, v {text_ty})",
        c.qt(&t),
        id_type(c)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, NULL), (2, ''), (3, ' '), (4, 'NULL'), (5, 'null'), (6, '0')",
        c.qt(&t)
    ))
    .await;
    let rows = c.page(&c.browse(&t), 0, 10).await.rows;
    let vals: Vec<Value> = rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(
        vals,
        vec![
            Value::Null,
            text(""),
            text(" "),
            text("NULL"),
            text("null"),
            text("0")
        ]
    );
    // Filters tell them apart.
    let mut req = c.browse(&t);
    req.filters = vec![filter("v", dbcore::model::FilterOp::IsNull, None)];
    assert_eq!(c.page_ids(&req, 0, 10).await, [1]);
    req.filters = vec![filter("v", dbcore::model::FilterOp::Eq, Some(""))];
    if c.is_mariadb() {
        // MariaDB's default collation is PAD SPACE: '' = ' ' there.
        assert_eq!(c.page_ids(&req, 0, 10).await, [2, 3]);
    } else {
        assert_eq!(c.page_ids(&req, 0, 10).await, [2]);
    }
    req.filters = vec![filter("v", dbcore::model::FilterOp::IsNotNull, None)];
    assert_eq!(c.page_ids(&req, 0, 10).await, [2, 3, 4, 5, 6]);

    // Editing between NULL and empty keeps them apart.
    let set = |input: CellInput, id: i64, original: Value| ChangeSet {
        table: c.tref(&t),
        changes: vec![RowChange::Update {
            key: vec![ColumnValue {
                column: "id".into(),
                value: int(id),
            }],
            original: vec![ColumnValue {
                column: "v".into(),
                value: original,
            }],
            set: vec![ColumnInput {
                column: "v".into(),
                value: input,
            }],
        }],
    };
    c.driver
        .apply_changes(&set(CellInput::Text(String::new()), 1, Value::Null))
        .await
        .unwrap();
    c.driver
        .apply_changes(&set(CellInput::Null, 2, text("")))
        .await
        .unwrap();
    let rows = c.page(&c.browse(&t), 0, 2).await.rows;
    assert_eq!(rows[0][1], text(""));
    assert_eq!(rows[1][1], Value::Null);
}
each_engine!(null_is_not_empty_text, sql);

async fn long_values_are_cut_for_the_grid_but_not_for_exports(c: &Ctx) {
    let t = c.scratch("long");
    let (text_ty, bytes_ty) = match c.engine {
        Engine::Postgres => ("text", "bytea"),
        Engine::Sqlite => ("TEXT", "BLOB"),
        _ => ("LONGTEXT", "LONGBLOB"),
    };
    c.side_exec(&format!(
        "CREATE TABLE {} (id {} PRIMARY KEY, s {text_ty}, b {bytes_ty})",
        c.qt(&t),
        id_type(c)
    ))
    .await;
    let blob = |n: usize| match c.engine {
        Engine::Postgres => format!("decode(repeat('ab', {n}), 'hex')"),
        Engine::Sqlite => format!("zeroblob({n})"),
        _ => format!("UNHEX(REPEAT('ab', {n}))"),
    };
    let rep = |s: &str, n: usize| match c.engine {
        Engine::Sqlite => format!("replace(hex(zeroblob({n})), '00', '{s}')"),
        _ => format!("REPEAT('{s}', {n})"),
    };
    // 1: exactly at the limits; 2: one over; 3: multi-byte characters at and over the limit.
    let rows = [
        (1, rep("x", 16384), blob(4096)),
        (2, rep("x", 16385), blob(4097)),
        (3, rep("🎉", 16384), blob(10)),
        (4, rep("🎉", 16385), blob(5000)),
        (5, rep("é", 20000), blob(0)),
    ];
    for (id, s, b) in &rows {
        c.side_exec(&format!("INSERT INTO {} VALUES ({id}, {s}, {b})", c.qt(&t)))
            .await;
    }
    let page = c.page(&c.browse(&t), 0, 10).await;
    let cell = |row: usize, col: usize| page.rows[row][col].clone();
    let is_text =
        |v: Value, chars: usize| matches!(v, Value::Text(s) if s.chars().count() == chars);
    let is_cut = |v: Value, total: usize| matches!(v, Value::TruncatedText { prefix, len } if len == total && prefix.chars().count() == 16384);
    assert!(
        is_text(cell(0, 1), 16384),
        "{:?}",
        cell(0, 1).to_display().len()
    );
    assert!(is_cut(cell(1, 1), 16385));
    assert!(is_text(cell(2, 1), 16384), "emoji at the limit stay whole");
    assert!(is_cut(cell(3, 1), 16385));
    assert!(is_cut(cell(4, 1), 20000));
    let blob_len = |v: Value| match v {
        Value::Bytes { data, len } => (data.len(), len),
        other => panic!("{other:?}"),
    };
    assert_eq!(blob_len(cell(0, 2)), (4096, 4096));
    assert_eq!(
        blob_len(cell(1, 2)),
        (4096, 4097),
        "bytes beyond 4 KiB are cut but sized"
    );
    assert_eq!(blob_len(cell(3, 2)), (4096, 5000));
    assert_eq!(blob_len(cell(4, 2)), (0, 0));
    assert!(cell(0, 2).is_complete() && !cell(1, 2).is_complete());
    // `query` truncates like the grid; `browse_all` returns everything.
    let events = c
        .run(&format!("SELECT s, b FROM {} WHERE id = 4", c.qt(&t)))
        .await;
    let set = &sets_of(&events)[0];
    assert!(is_cut(set.rows[0][0].clone(), 16385));
    assert_eq!(blob_len(set.rows[0][1].clone()), (4096, 5000));
    let mut req = c.browse(&t);
    req.filters = vec![filter("id", dbcore::model::FilterOp::Eq, Some("4"))];
    let full = c.all_rows(&req).await;
    assert_eq!(full.len(), 1);
    assert!(is_text(full[0][1].clone(), 16385), "browse_all is complete");
    assert_eq!(blob_len(full[0][2].clone()), (5000, 5000));
    // A truncated value cannot be used as a key or as an `original`: edits go ahead without it.
    let changes = ChangeSet {
        table: c.tref(&t),
        changes: vec![RowChange::Update {
            key: vec![ColumnValue {
                column: "id".into(),
                value: int(4),
            }],
            original: vec![
                ColumnValue {
                    column: "s".into(),
                    value: cell(3, 1),
                },
                ColumnValue {
                    column: "b".into(),
                    value: cell(3, 2),
                },
            ],
            set: vec![ColumnInput {
                column: "s".into(),
                value: CellInput::Text("short".into()),
            }],
        }],
    };
    let sql = c.driver.preview_changes(&changes).await.unwrap();
    assert!(
        !sql[0].contains("🎉"),
        "truncated text must not reach the WHERE clause"
    );
    assert_eq!(
        c.driver
            .apply_changes(&changes)
            .await
            .unwrap()
            .rows_affected,
        1
    );
    assert_eq!(
        c.scalar(&format!("SELECT s FROM {} WHERE id = 4", c.qt(&t)))
            .await,
        text("short")
    );
    // Using a truncated value as a key is refused outright.
    let as_key = ChangeSet {
        table: c.tref(&t),
        changes: vec![RowChange::Delete {
            key: vec![
                ColumnValue {
                    column: "id".into(),
                    value: int(2),
                },
                ColumnValue {
                    column: "s".into(),
                    value: cell(1, 1),
                },
            ],
        }],
    };
    assert!(matches!(
        c.driver.preview_changes(&as_key).await,
        Err(dbcore::Error::InvalidInput(m)) if m.contains("too long")
    ));
}
each_engine!(long_values_are_cut_for_the_grid_but_not_for_exports, sql);

async fn pg_timestamptz_follows_the_session_time_zone(c: &Ctx) {
    let t = c.scratch("tz");
    c.side_exec(&format!(
        "CREATE TABLE {} (id integer PRIMARY KEY, tz timestamptz, plain timestamp, t timetz)",
        c.qt(&t)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES
            (1, '2024-07-01 12:00:00+00', '2024-07-01 12:00:00', '12:00:00+00'),
            (2, '2024-01-15 12:00:00+00', '2024-01-15 12:00:00', '12:00:00+00'),
            (3, '1900-01-01 00:00:00+00', '1900-01-01 00:00:00', '00:00:00+00'),
            (4, '2024-03-10 06:30:00+00', '2024-03-10 06:30:00', '06:30:00+00')",
        c.qt(&t)
    ))
    .await;
    // (zone, expected rendering of rows 1..4 for the timestamptz column)
    let zones: [(&str, [&str; 4]); 4] = [
        (
            "UTC",
            [
                "2024-07-01 12:00:00+00",
                "2024-01-15 12:00:00+00",
                "1900-01-01 00:00:00+00",
                "2024-03-10 06:30:00+00",
            ],
        ),
        (
            "America/New_York",
            [
                "2024-07-01 08:00:00-04",
                "2024-01-15 07:00:00-05",
                "1899-12-31 19:00:00-05",
                "2024-03-10 01:30:00-05",
            ],
        ),
        (
            "Asia/Kolkata",
            [
                "2024-07-01 17:30:00+05:30",
                "2024-01-15 17:30:00+05:30",
                "1900-01-01 05:53:28+05:53:28",
                "2024-03-10 12:00:00+05:30",
            ],
        ),
        (
            "Pacific/Chatham",
            [
                "2024-07-02 00:45:00+12:45",
                "2024-01-16 01:45:00+13:45",
                "1900-01-01 00:00:00+00",
                "2024-03-10 20:15:00+13:45",
            ],
        ),
    ];
    for (zone, expected) in zones {
        let driver = connect_driver(Engine::Postgres, None, |cfg| {
            cfg.options.insert("timezone".into(), zone.into());
        })
        .await
        .unwrap();
        let page = driver.browse_page(&c.browse(&t), 0, 10).await.unwrap();
        for (i, want) in expected.iter().enumerate() {
            let got = page.rows[i][1].clone();
            if zone == "Pacific/Chatham" && i == 2 || zone == "Asia/Kolkata" && i == 2 {
                // Historic local mean time: only check that it is the same instant.
                let instant = drain(
                    driver
                        .query(
                            None,
                            &format!(
                                "SELECT tz = '1900-01-01 00:00:00+00' FROM {} WHERE id = 3",
                                c.qt(&t)
                            ),
                        )
                        .await
                        .unwrap(),
                )
                .await
                .unwrap();
                assert_eq!(sets_of(&instant)[0].rows[0][0], Value::Bool(true), "{zone}");
                continue;
            }
            assert_eq!(got, datetime(want), "timestamptz row {} in {zone}", i + 1);
        }
        // `timestamp` and `timetz` literals do not move.
        assert_eq!(page.rows[0][2], datetime("2024-07-01 12:00:00"), "{zone}");
        assert_eq!(page.rows[2][2], datetime("1900-01-01 00:00:00"), "{zone}");
        assert_eq!(page.rows[0][3], time("12:00:00+00"), "{zone}");
        // Typing a local time stores the right instant.
        driver
            .apply_changes(&ChangeSet {
                table: c.tref(&t),
                changes: vec![RowChange::Update {
                    key: vec![ColumnValue {
                        column: "id".into(),
                        value: int(4),
                    }],
                    original: vec![],
                    set: vec![ColumnInput {
                        column: "tz".into(),
                        value: CellInput::Text("2024-06-01 09:00:00".into()),
                    }],
                }],
            })
            .await
            .unwrap();
        let stored = c
            .side_text(&format!(
                "SELECT (tz AT TIME ZONE 'UTC')::text FROM {} WHERE id = 4",
                c.qt(&t)
            ))
            .await
            .unwrap();
        let utc = match zone {
            "UTC" => "2024-06-01 09:00:00",
            "America/New_York" => "2024-06-01 13:00:00",
            "Asia/Kolkata" => "2024-06-01 03:30:00",
            _ => "2024-05-31 20:15:00",
        };
        assert_eq!(stored, utc, "9:00 local time in {zone}");
        driver.close().await;
        c.side_exec(&format!(
            "UPDATE {} SET tz = '2024-03-10 06:30:00+00' WHERE id = 4",
            c.qt(&t)
        ))
        .await;
    }
}
each_engine!(pg_timestamptz_follows_the_session_time_zone, postgres);

async fn mysql_timestamp_follows_the_session_time_zone(c: &Ctx) {
    let t = c.scratch("tz");
    c.side_exec(&format!(
        "CREATE TABLE {} (id INT PRIMARY KEY, ts TIMESTAMP NULL, dt DATETIME)",
        c.qt(&t)
    ))
    .await;
    // The side channel's session zone is the server's (UTC in docker).
    c.side_exec("SET SESSION time_zone = '+00:00'").await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, '2024-07-01 12:00:00', '2024-07-01 12:00:00'), (2, '1970-01-01 00:00:01', '1970-01-01 00:00:01')",
        c.qt(&t)
    ))
    .await;
    for (zone, ts1, ts2) in [
        ("+00:00", "2024-07-01 12:00:00", "1970-01-01 00:00:01"),
        ("+05:30", "2024-07-01 17:30:00", "1970-01-01 05:30:01"),
        ("-08:00", "2024-07-01 04:00:00", "1969-12-31 16:00:01"),
    ] {
        let driver = connect_driver(c.engine, None, |cfg| {
            cfg.options.insert("timezone".into(), zone.into());
        })
        .await
        .unwrap();
        let page = driver.browse_page(&c.browse(&t), 0, 10).await.unwrap();
        assert_eq!(page.rows[0][1], datetime(ts1), "TIMESTAMP in {zone}");
        assert_eq!(page.rows[1][1], datetime(ts2), "TIMESTAMP in {zone}");
        // DATETIME does not move.
        assert_eq!(
            page.rows[0][2],
            datetime("2024-07-01 12:00:00"),
            "DATETIME in {zone}"
        );
        // Writing a local time stores the right instant.
        driver
            .apply_changes(&ChangeSet {
                table: c.tref(&t),
                changes: vec![RowChange::Update {
                    key: vec![ColumnValue {
                        column: "id".into(),
                        value: int(1),
                    }],
                    original: vec![],
                    set: vec![ColumnInput {
                        column: "ts".into(),
                        value: CellInput::Text(ts1.into()),
                    }],
                }],
            })
            .await
            .unwrap();
        assert_eq!(
            c.side_text(&format!(
                "SELECT CAST(ts AS CHAR) FROM {} WHERE id = 1",
                c.qt(&t)
            ))
            .await
            .as_deref(),
            Some("2024-07-01 12:00:00"),
            "round trip in {zone}"
        );
        driver.close().await;
    }
}
each_engine!(
    mysql_timestamp_follows_the_session_time_zone,
    mysql,
    mariadb
);

async fn seeded_all_types_tables_decode(c: &Ctx) {
    // Read-only: the nasty `all_types` table of the docker databases decodes row by row.
    let t = c.tref("all_types");
    let s = c.driver.table_structure(&t).await.unwrap();
    let page = c.page(&c.browse("all_types"), 0, 50).await;
    assert_eq!(page.columns.len(), s.columns.len());
    assert!(page.rows.len() >= 3);
    // Row 2 is all NULL apart from the key.
    assert!(
        page.rows[1].iter().skip(1).all(Value::is_null),
        "{:?}",
        page.rows[1]
    );
    for row in &page.rows {
        assert_eq!(row.len(), page.columns.len());
    }
    let idx = |n: &str| {
        page.columns
            .iter()
            .position(|c| c.name == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    if c.is_pg() {
        assert_eq!(page.rows[0][idx("c_bigint")], int(9_007_199_254_740_993));
        assert_eq!(page.rows[2][idx("c_bigint")], int(i64::MIN));
        assert_eq!(page.rows[2][idx("c_double")], float(f64::INFINITY));
        assert_eq!(page.rows[2][idx("c_real")], float(f64::NEG_INFINITY));
        assert!(matches!(&page.rows[2][idx("c_numeric_free")], Value::Decimal(s) if s == "NaN"));
        assert_eq!(page.rows[2][idx("c_timestamp")], datetime("-infinity"));
        assert_eq!(page.rows[0][idx("c_mood")], text("happy"));
        assert!(
            matches!(&page.rows[0][idx("c_int_array")], Value::Other { type_name, text } if type_name == "int4[]" && text == "{1,2,3}")
        );
    }
}
each_engine!(seeded_all_types_tables_decode, postgres, mysql, mariadb);

async fn pg_text_cannot_hold_nul(c: &Ctx) {
    // Postgres text has no NUL character: the attempt fails cleanly and nothing is half-written.
    let t = c.scratch("nul");
    c.side_exec(&format!(
        "CREATE TABLE {} (id integer PRIMARY KEY, v text)",
        c.qt(&t)
    ))
    .await;
    let set = |value: &str| ChangeSet {
        table: c.tref(&t),
        changes: vec![
            RowChange::Insert {
                values: vec![
                    ColumnInput {
                        column: "id".into(),
                        value: CellInput::Text("1".into()),
                    },
                    ColumnInput {
                        column: "v".into(),
                        value: CellInput::Text(value.into()),
                    },
                ],
            },
            RowChange::Insert {
                values: vec![ColumnInput {
                    column: "id".into(),
                    value: CellInput::Text("2".into()),
                }],
            },
        ],
    };
    let err = c
        .driver
        .apply_changes(&set("before\0after; DROP TABLE x; --"))
        .await;
    assert!(err.is_err(), "{err:?}");
    assert_eq!(c.side_count(&t).await, 0, "the whole set is rolled back");
    // The connection is not left in a broken state.
    assert!(c.driver.apply_changes(&set("fine")).await.is_ok());
    assert_eq!(c.side_count(&t).await, 2);
}
each_engine!(pg_text_cannot_hold_nul, postgres);
