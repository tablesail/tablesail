//! Export (CSV, JSON, JSON lines, SQL) and CSV import against every SQL engine.

#[macro_use]
mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use dbcore::export::{CancellationToken, ProgressFn, csv_preview, export, import_csv};
use dbcore::model::{CsvImport, ExportFormat, ExportSource, FilterOp, JobProgress};
use dbcore::{Engine, Error, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A scratch directory removed at the end of the test.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!("tablesoft-it-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

struct Recorder {
    seen: Arc<Mutex<Vec<JobProgress>>>,
    callback: ProgressFn,
}

fn recorder() -> Recorder {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    Recorder {
        seen,
        callback: Arc::new(move |p| sink.lock().unwrap().push(p.clone())),
    }
}

impl Recorder {
    fn all(&self) -> Vec<JobProgress> {
        self.seen.lock().unwrap().clone()
    }
}

async fn export_to(
    c: &Ctx,
    source: ExportSource,
    format: ExportFormat,
    path: &Path,
) -> dbcore::Result<JobProgress> {
    export(
        c.driver.clone(),
        source,
        format,
        path.to_path_buf(),
        recorder().callback,
        CancellationToken::new(),
    )
    .await
}

fn table_source(c: &Ctx, name: &str) -> ExportSource {
    ExportSource::Table {
        request: c.browse(name),
    }
}

fn csv_format(delimiter: char, header: bool, null_as: &str) -> ExportFormat {
    ExportFormat::Csv {
        delimiter,
        header,
        null_as: null_as.to_owned(),
    }
}

fn read_csv(path: &Path, delimiter: u8) -> Vec<Vec<String>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .from_path(path)
        .unwrap();
    reader
        .records()
        .map(|r| r.unwrap().iter().map(str::to_owned).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const COLUMNS: [&str; 10] = [
    "id", "name", "amount", "ratio", "flag", "big", "born", "seen", "doc", "raw",
];

fn export_ddl(c: &Ctx, table: &str) -> String {
    let t = c.qt(table);
    match c.engine {
        Engine::Postgres => format!(
            "CREATE TABLE {t} (id integer PRIMARY KEY, name text, amount numeric(12,3), ratio double precision, flag boolean, big bigint, born date, seen timestamp, doc jsonb, raw bytea)"
        ),
        Engine::Sqlite => format!(
            "CREATE TABLE {t} (id INTEGER PRIMARY KEY, name TEXT, amount NUMERIC, ratio REAL, flag BOOLEAN, big INTEGER, born DATE, seen DATETIME, doc JSON, raw BLOB)"
        ),
        _ => format!(
            "CREATE TABLE {t} (id INT PRIMARY KEY, name VARCHAR(200), amount DECIMAL(12,3), ratio DOUBLE, flag TINYINT(1), big BIGINT, born DATE, seen DATETIME, doc JSON, raw BLOB)"
        ),
    }
}

/// The names the fixture's rows have; `None` is SQL NULL.
fn names() -> Vec<Option<&'static str>> {
    vec![
        Some("plain"),
        Some("with,comma"),
        Some("quote \"inside\" and 'single'"),
        Some("multi\nline\r\nvalue"),
        Some("ünï 日本語 🎉"),
        Some("semi;colon\ttab|pipe"),
        Some("  padded  "),
        None,
        Some(""),
        Some("back\\slash"),
    ]
}

async fn export_fixture(c: &Ctx, suffix: &str) -> String {
    let name = c.scratch(suffix);
    c.side_exec(&export_ddl(c, &name)).await;
    let t = c.qt(&name);
    let (yes, no) = if c.is_pg() {
        ("true", "false")
    } else {
        ("1", "0")
    };
    let bytes = |hex: &str| {
        if c.is_pg() {
            format!("'\\x{hex}'")
        } else {
            format!("X'{hex}'")
        }
    };
    for (i, n) in names().iter().enumerate() {
        let id = i + 1;
        let name = n.map_or("NULL".to_owned(), |s| c.lit(s));
        let row = if id == 8 {
            format!("({id}, {name}, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)")
        } else {
            let flag = if id % 2 == 1 { yes } else { no };
            let big = if id == 2 {
                "-9223372036854775808".to_owned()
            } else {
                (9_007_199_254_740_993_i64 + id as i64).to_string()
            };
            let amount = format!("{}.{}", id as i64 * 1111 - 5000, 100 + id);
            let doc = c.lit(&format!("{{\"a\":[1,{id}],\"s\":\"x\"}}"));
            let raw = bytes(&format!("dead{id:02x}"));
            format!(
                "({id}, {name}, {amount}, {id}.5, {flag}, {big}, '2024-02-{id:02}', '2024-02-{id:02} 13:14:15', {doc}, {raw})"
            )
        };
        c.side_exec(&format!("INSERT INTO {t} VALUES {row}")).await;
    }
    name
}

// ---------------------------------------------------------------------------
// CSV export
// ---------------------------------------------------------------------------

async fn csv_export_quotes_and_nulls(c: &Ctx) {
    let t = export_fixture(c, "csv").await;
    let dir = TempDir::new();
    let path = dir.file("out.csv");
    let rec = recorder();
    let done = export(
        c.driver.clone(),
        table_source(c, &t),
        csv_format(',', true, "\\N"),
        path.clone(),
        rec.callback.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(done.done && !done.cancelled && done.error.is_none() && done.warning.is_none());
    assert_eq!(done.rows, 10);
    assert_eq!(done.bytes, std::fs::metadata(&path).unwrap().len());
    assert_eq!(
        rec.all().last().unwrap(),
        &done,
        "the last progress report is the result"
    );

    let records = read_csv(&path, b',');
    assert_eq!(records.len(), 11);
    assert_eq!(records[0], COLUMNS);
    // Awkward text survives CSV quoting exactly.
    let name_of = |id: usize| records[id][1].clone();
    assert_eq!(name_of(1), "plain");
    assert_eq!(name_of(2), "with,comma");
    assert_eq!(name_of(3), "quote \"inside\" and 'single'");
    assert_eq!(name_of(4), "multi\nline\r\nvalue");
    assert_eq!(name_of(5), "ünï 日本語 🎉");
    assert_eq!(name_of(6), "semi;colon\ttab|pipe");
    assert_eq!(name_of(7), "  padded  ");
    assert_eq!(name_of(8), "\\N", "NULL is written as the chosen marker");
    assert_eq!(
        name_of(9),
        "",
        "an empty string stays empty and is not NULL"
    );
    assert_eq!(name_of(10), "back\\slash");
    // Other types as the grid would show them.
    assert_eq!(records[1][0], "1");
    assert_eq!(records[1][5], "9007199254740994");
    assert_eq!(records[2][5], "-9223372036854775808");
    assert_eq!(records[1][6], "2024-02-01");
    assert!(
        records[1][7].starts_with("2024-02-01 13:14:15"),
        "{}",
        records[1][7]
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&records[1][8]).unwrap(),
        serde_json::json!({"a": [1, 1], "s": "x"})
    );
    assert_eq!(records[1][9], "0xdead01");
    assert!(
        records[8][2..].iter().all(|f| f == "\\N"),
        "row 8 is NULL everywhere: {:?}",
        records[8]
    );

    // Booleans export the way the database stores them, so the file can be imported again.
    if c.is_my() || c.is_sqlite() {
        assert_eq!((records[1][4].as_str(), records[2][4].as_str()), ("1", "0"));
    } else {
        assert_eq!(
            (records[1][4].as_str(), records[2][4].as_str()),
            ("true", "false")
        );
    }
}
each_engine!(csv_export_quotes_and_nulls, sql);

async fn csv_export_options(c: &Ctx) {
    let t = export_fixture(c, "csv_opts").await;
    let dir = TempDir::new();
    // Default NULL marker is empty, header can be dropped, delimiters vary.
    for (delimiter, byte) in [(';', b';'), ('\t', b'\t'), ('|', b'|'), (',', b',')] {
        let path = dir.file(&format!("d{byte}.csv"));
        let done = export_to(
            c,
            table_source(c, &t),
            csv_format(delimiter, false, ""),
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 10);
        let records = read_csv(&path, byte);
        assert_eq!(records.len(), 10, "no header row for {delimiter:?}");
        assert_eq!(records[0].len(), COLUMNS.len());
        assert_eq!(
            records[7][1], "",
            "NULL and '' are both empty with the default marker"
        );
        assert_eq!(records[8][1], "");
        assert_eq!(records[2][1], "quote \"inside\" and 'single'");
        assert_eq!(records[3][1], "multi\nline\r\nvalue");
    }
    // Delimiters that would break the format are refused up front.
    for bad in ['"', '\n', '\r', 'é', '🎉'] {
        let path = dir.file("bad.csv");
        let err = export_to(c, table_source(c, &t), csv_format(bad, true, ""), &path)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{bad:?}: {err:?}");
    }
    // Filter, sort and column choice apply to the export.
    let mut req = c.browse(&t);
    req.columns = vec!["id".into(), "name".into()];
    req.filters = vec![filter("id", FilterOp::Lte, Some("5"))];
    req.sort = vec![sort("id", true)];
    let path = dir.file("subset.csv");
    export_to(
        c,
        ExportSource::Table { request: req },
        csv_format(',', true, ""),
        &path,
    )
    .await
    .unwrap();
    let records = read_csv(&path, b',');
    assert_eq!(records[0], ["id", "name"]);
    assert_eq!(
        records
            .iter()
            .skip(1)
            .map(|r| r[0].as_str())
            .collect::<Vec<_>>(),
        ["5", "4", "3", "2", "1"]
    );
}
each_engine!(csv_export_options, sql);

// ---------------------------------------------------------------------------
// JSON, JSON lines
// ---------------------------------------------------------------------------

async fn json_exports_parse_back(c: &Ctx) {
    let t = export_fixture(c, "json").await;
    let dir = TempDir::new();
    let expected_rows = c.all_rows(&c.browse(&t)).await;
    let mut expected_rows = expected_rows;
    expected_rows.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);

    for format in [
        ExportFormat::Json { pretty: false },
        ExportFormat::Json { pretty: true },
    ] {
        let path = dir.file("out.json");
        let done = export_to(c, table_source(c, &t), format, &path)
            .await
            .unwrap();
        assert_eq!(done.rows, 10);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let array = parsed.as_array().unwrap();
        assert_eq!(array.len(), 10);
        check_json_objects(c, array, &expected_rows);
    }
    let path = dir.file("out.jsonl");
    let done = export_to(c, table_source(c, &t), ExportFormat::JsonLines, &path)
        .await
        .unwrap();
    assert_eq!(done.rows, 10);
    let text = std::fs::read_to_string(&path).unwrap();
    let objects: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        objects.len(),
        10,
        "one object per line, even with newlines inside values"
    );
    check_json_objects(c, &objects, &expected_rows);

    // An empty table is an empty array / an empty file.
    let empty = c.make_items("json_empty", 0).await;
    let path = dir.file("empty.json");
    let done = export_to(
        c,
        table_source(c, &empty),
        ExportFormat::Json { pretty: true },
        &path,
    )
    .await
    .unwrap();
    assert_eq!(done.rows, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
            .unwrap(),
        serde_json::json!([])
    );
    let path = dir.file("empty.jsonl");
    export_to(c, table_source(c, &empty), ExportFormat::JsonLines, &path)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    let path = dir.file("empty.csv");
    export_to(c, table_source(c, &empty), csv_format(',', true, ""), &path)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "id,name,grp,label\n",
        "header only"
    );
}
each_engine!(json_exports_parse_back, sql);

fn check_json_objects(c: &Ctx, objects: &[serde_json::Value], rows: &[Row]) {
    use serde_json::Value as J;
    for (obj, row) in objects.iter().zip(rows) {
        let obj = obj.as_object().expect("an object per row");
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(keys, COLUMNS, "keys keep column order");
        let id = ids(std::slice::from_ref(row), 0)[0];
        assert_eq!(obj["id"], J::from(id));
        // Text and NULL.
        match &row[1] {
            Value::Null => assert_eq!(obj["name"], J::Null, "row {id}"),
            Value::Text(s) => assert_eq!(obj["name"], J::from(s.clone()), "row {id}"),
            other => panic!("{other:?}"),
        }
        if id == 8 {
            assert!(COLUMNS[2..].iter().all(|k| obj[*k] == J::Null), "{obj:?}");
            continue;
        }
        // Integers beyond 2^53 are strings so no precision is lost.
        match &row[5] {
            Value::Int(i) if i.unsigned_abs() > 9_007_199_254_740_991 => {
                assert_eq!(obj["big"], J::from(i.to_string()), "row {id}")
            }
            other => panic!("{other:?}"),
        }
        // Decimals are strings (SQLite stores them as numbers).
        match &row[2] {
            Value::Decimal(d) => assert_eq!(obj["amount"], J::from(d.clone())),
            Value::Float(f) => assert_eq!(obj["amount"].as_f64().unwrap(), *f),
            Value::Int(i) => assert_eq!(obj["amount"], J::from(*i)),
            other => panic!("{other:?}"),
        }
        assert_eq!(obj["ratio"].as_f64().unwrap(), (id as f64) + 0.5);
        // Booleans are booleans where the database has them, 0/1 where it does not.
        match &row[4] {
            Value::Bool(b) => assert_eq!(obj["flag"], J::from(*b)),
            Value::Int(i) => assert_eq!(obj["flag"], J::from(*i)),
            other => panic!("{other:?}"),
        }
        assert_eq!(obj["born"], J::from(format!("2024-02-{id:02}")));
        assert!(
            obj["seen"]
                .as_str()
                .unwrap()
                .starts_with(&format!("2024-02-{id:02} 13:14:15"))
        );
        // JSON values nest; SQLite and MariaDB hold it as text.
        if c.is_sqlite() || c.is_mariadb() {
            assert!(obj["doc"].is_string());
        } else {
            assert_eq!(
                obj["doc"],
                serde_json::json!({"a": [1, id], "s": "x"}),
                "row {id}"
            );
        }
        // Bytes are base64.
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(obj["raw"].as_str().unwrap())
            .unwrap();
        assert_eq!(raw, vec![0xde, 0xad, id as u8]);
    }
}

// ---------------------------------------------------------------------------
// SQL export, then run it
// ---------------------------------------------------------------------------

async fn sql_export_recreates_the_table(c: &Ctx) {
    let t = export_fixture(c, "sql_src").await;
    let dir = TempDir::new();
    let copy = c.scratch("sql_copy");
    // A name that needs quoting.
    let target = format!("{copy} \"odd\" name");
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&target)));
    for rows_per_insert in [3u32, 1, 100] {
        let path = dir.file("dump.sql");
        let done = export_to(
            c,
            table_source(c, &t),
            ExportFormat::Sql {
                table: target.clone(),
                rows_per_insert,
            },
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 10);
        let sql = std::fs::read_to_string(&path).unwrap();
        let inserts = sql.matches("INSERT INTO").count();
        assert_eq!(
            inserts as u32,
            10u32.div_ceil(rows_per_insert),
            "{rows_per_insert} rows per INSERT"
        );
        assert!(
            sql.starts_with(&format!("INSERT INTO {} (", c.q(&target))),
            "{}",
            &sql[..80.min(sql.len())]
        );
        assert!(sql.contains(&c.q("name")));

        // Recreate the table (same shape) and replay the file through the driver.
        c.side_exec(&format!("DROP TABLE IF EXISTS {}", c.qt(&target)))
            .await;
        c.side_exec(&export_ddl(c, &target)).await;
        let results = run_script(c, &sql).await;
        assert_eq!(results.len(), inserts);
        for (statement, result) in &results {
            result
                .as_ref()
                .unwrap_or_else(|e| panic!("{e:?}\n{}", &statement[..statement.len().min(400)]));
        }
        let mut original = c.all_rows(&c.browse(&t)).await;
        let mut copied = c.all_rows(&c.browse(&target)).await;
        original.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
        copied.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
        assert_eq!(copied.len(), 10);
        for (a, b) in original.iter().zip(&copied) {
            assert_eq!(a, b, "{rows_per_insert} rows per INSERT");
        }
    }
    // An unusable table name is refused.
    let path = dir.file("bad.sql");
    let err = export_to(
        c,
        table_source(c, &t),
        ExportFormat::Sql {
            table: "  ".into(),
            rows_per_insert: 10,
        },
        &path,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
}
each_engine!(sql_export_recreates_the_table, sql);

async fn sql_export_of_extreme_numbers(c: &Ctx) {
    // Numbers at the edges survive export -> replay.
    let src = c.scratch("sql_nums");
    let copy = c.scratch("sql_nums_copy");
    let (int, float, dec) = match c.engine {
        Engine::Postgres => ("integer", "double precision", "numeric(40,20)"),
        Engine::Sqlite => ("INTEGER", "REAL", "NUMERIC"),
        _ => ("INT", "DOUBLE", "DECIMAL(40,20)"),
    };
    let ddl = |t: &str| {
        format!(
            "CREATE TABLE {} (id {int} PRIMARY KEY, f {float}, d {dec})",
            c.qt(t)
        )
    };
    c.side_exec(&ddl(&src)).await;
    c.side_exec(&ddl(&copy)).await;
    let mut rows = vec![
        (
            "1",
            "1.7976931348623157e308",
            "12345678901234567890.12345678901234567890",
        ),
        ("2", "5e-324", "-0.00000000000000000001"),
        ("3", "-1.5e-10", "0"),
        ("4", "123456789012345678", "100.5"),
        ("5", "0.30000000000000004", "1"),
    ];
    if c.is_my() {
        rows[1].1 = "1e-300"; // MySQL has no denormal doubles
    }
    for (id, f, d) in &rows {
        c.side_exec(&format!(
            "INSERT INTO {} VALUES ({id}, {f}, {d})",
            c.qt(&src)
        ))
        .await;
    }
    let dir = TempDir::new();
    let path = dir.file("nums.sql");
    export_to(
        c,
        table_source(c, &src),
        ExportFormat::Sql {
            table: copy.clone(),
            rows_per_insert: 100,
        },
        &path,
    )
    .await
    .unwrap();
    let sql = std::fs::read_to_string(&path).unwrap();
    // The rows land in a table with the other name: point the INSERT at its qualified name.
    let sql = sql.replace(&c.q(&copy), &c.qt(&copy));
    for (statement, result) in run_script(c, &sql).await {
        result.unwrap_or_else(|e| panic!("{e:?}\n{statement}"));
    }
    let mut a = c.all_rows(&c.browse(&src)).await;
    let mut b = c.all_rows(&c.browse(&copy)).await;
    a.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    b.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    assert_eq!(a, b);
}
each_engine!(sql_export_of_extreme_numbers, sql);

async fn sql_export_of_nan_and_infinity(c: &Ctx) {
    // Where the database can hold them, special floats are written in a form it reads back.
    let src = c.scratch("sql_special");
    let copy = c.scratch("sql_special_copy");
    let (int, float) = if c.is_pg() {
        ("integer", "double precision")
    } else {
        ("INTEGER", "REAL")
    };
    for t in [&src, &copy] {
        c.side_exec(&format!(
            "CREATE TABLE {} (id {int} PRIMARY KEY, f {float})",
            c.qt(t)
        ))
        .await;
    }
    let infinity = if c.is_pg() { "'Infinity'" } else { "9e999" };
    let neg_infinity = if c.is_pg() { "'-Infinity'" } else { "-9e999" };
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, {infinity}), (2, {neg_infinity}), (3, 1.5)",
        c.qt(&src)
    ))
    .await;
    if c.is_pg() {
        c.side_exec(&format!("INSERT INTO {} VALUES (4, 'NaN')", c.qt(&src)))
            .await;
    }
    let dir = TempDir::new();
    let path = dir.file("special.sql");
    export_to(
        c,
        table_source(c, &src),
        ExportFormat::Sql {
            table: copy.clone(),
            rows_per_insert: 100,
        },
        &path,
    )
    .await
    .unwrap();
    let sql = std::fs::read_to_string(&path)
        .unwrap()
        .replace(&c.q(&copy), &c.qt(&copy));
    for (statement, result) in run_script(c, &sql).await {
        result.unwrap_or_else(|e| panic!("{e:?}\n{statement}"));
    }
    let shown = |t: &str| {
        let sql = format!("SELECT id, f FROM {} ORDER BY id", c.qt(t));
        async move {
            c.rows(&sql)
                .await
                .iter()
                .map(|r| format!("{}={}", r[0].to_display(), r[1].to_display()))
                .collect::<Vec<_>>()
        }
    };
    let (a, b) = (shown(&src).await, shown(&copy).await);
    assert_eq!(a, b);
    assert!(a[0].contains("inf") || a[0].contains("Inf"), "{a:?}");
}
each_engine!(sql_export_of_nan_and_infinity, postgres, sqlite);

// ---------------------------------------------------------------------------
// Query exports, progress, errors and cancelling
// ---------------------------------------------------------------------------

async fn query_exports_and_truncation_warnings(c: &Ctx) {
    let t = c.make_items("qexp", 1500).await;
    let dir = TempDir::new();
    let path = dir.file("q.csv");
    let done = export_to(
        c,
        ExportSource::Query {
            database: Some(c.database.clone()),
            sql: format!(
                "SELECT id, name FROM {} WHERE grp = 2 ORDER BY id DESC",
                c.qt(&t)
            ),
        },
        csv_format(',', true, ""),
        &path,
    )
    .await
    .unwrap();
    let want: Vec<i64> = (1..=1500).filter(|i| item(*i).grp == 2).rev().collect();
    assert_eq!(done.rows, want.len() as u64);
    let records = read_csv(&path, b',');
    assert_eq!(records[0], ["id", "name"]);
    assert_eq!(
        records[1..]
            .iter()
            .map(|r| r[0].parse::<i64>().unwrap())
            .collect::<Vec<_>>(),
        want
    );
    assert!(done.warning.is_none());

    // A query export goes through the grid path: long text is cut, and the job says so.
    let long = c.scratch("qexp_long");
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "LONGTEXT"),
    };
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY, v {text})",
        c.qt(&long)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, {})",
        c.qt(&long),
        c.lit(&"é".repeat(20_000))
    ))
    .await;
    let path = dir.file("long.jsonl");
    let done = export_to(
        c,
        ExportSource::Query {
            database: Some(c.database.clone()),
            sql: format!("SELECT id, v FROM {}", c.qt(&long)),
        },
        ExportFormat::JsonLines,
        &path,
    )
    .await
    .unwrap();
    assert!(
        done.warning
            .as_deref()
            .is_some_and(|w| w.contains("cut short")),
        "{done:?}"
    );
    let line: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
    assert_eq!(line["v"].as_str().unwrap().chars().count(), 16 * 1024);
    // The table export of the same data is complete and has no warning.
    let path = dir.file("long_table.jsonl");
    let done = export_to(c, table_source(c, &long), ExportFormat::JsonLines, &path)
        .await
        .unwrap();
    assert!(done.warning.is_none());
    let line: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
    assert_eq!(line["v"].as_str().unwrap().chars().count(), 20_000);
}
each_engine!(query_exports_and_truncation_warnings, sql);

async fn progress_is_reported_and_files_are_complete(c: &Ctx) {
    let n = 20_000;
    let t = c.make_items("progress", n).await;
    let dir = TempDir::new();
    let path = dir.file("big.csv");
    let rec = recorder();
    let done = export(
        c.driver.clone(),
        table_source(c, &t),
        csv_format(',', true, ""),
        path.clone(),
        rec.callback.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(done.rows, n as u64);
    let reports = rec.all();
    assert!(!reports.is_empty());
    for pair in reports.windows(2) {
        assert!(
            pair[0].rows <= pair[1].rows && pair[0].bytes <= pair[1].bytes,
            "progress never goes backwards"
        );
    }
    assert!(reports.last().unwrap().done);
    assert!(reports[..reports.len() - 1].iter().all(|p| !p.done));
    let records = read_csv(&path, b',');
    assert_eq!(records.len(), n as usize + 1);
    assert_eq!(records[n as usize][0], n.to_string());
    assert_eq!(done.bytes, std::fs::metadata(&path).unwrap().len());
}
each_engine!(progress_is_reported_and_files_are_complete, sql);

async fn export_failures_are_reported(c: &Ctx) {
    let t = c.make_items("exp_fail", 20).await;
    let dir = TempDir::new();
    // Missing directory.
    let err = export_to(
        c,
        table_source(c, &t),
        csv_format(',', true, ""),
        &dir.file("no/such/dir/out.csv"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Io(_)), "{err:?}");
    // A query that fails to start.
    let rec = recorder();
    let err = export(
        c.driver.clone(),
        ExportSource::Query {
            database: Some(c.database.clone()),
            sql: "SELECT * FROM no_such_table_xyz".into(),
        },
        csv_format(',', true, ""),
        dir.file("a.csv"),
        rec.callback.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    // A table that does not exist.
    let err = export_to(
        c,
        table_source(c, "no_such_table_xyz"),
        csv_format(',', true, ""),
        &dir.file("b.csv"),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, Error::NotFound(_) | Error::Database { .. }),
        "{err:?}"
    );
    // A filter the server rejects.
    let mut req = c.browse(&t);
    req.raw_where = Some("id = = 1".into());
    let err = export_to(
        c,
        ExportSource::Table { request: req },
        csv_format(',', true, ""),
        &dir.file("c.csv"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");

    // An error in the middle: rows already written stay, the progress report carries the message.
    if c.is_pg() {
        let rec = recorder();
        let err = export(
            c.driver.clone(),
            ExportSource::Query {
                database: None,
                sql: "SELECT 100 / (5 - g) AS v FROM (SELECT generate_series(1, 10) AS g) s".into(),
            },
            csv_format(',', true, ""),
            dir.file("mid.csv"),
            rec.callback.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, Error::Database { code: Some(code), .. } if code == "22012"),
            "{err:?}"
        );
        let last = rec.all().last().cloned().unwrap();
        assert!(
            last.done
                && last
                    .error
                    .as_deref()
                    .is_some_and(|m| m.contains("division by zero")),
            "{last:?}"
        );
    }
}
each_engine!(export_failures_are_reported, sql);

async fn cancelling_an_export_stops_it_quickly(c: &Ctx) {
    let dir = TempDir::new();
    let path = dir.file("cancelled.csv");
    let marker = c.name("exp_cancel");
    let sql = match c.engine {
        Engine::Postgres => format!(
            "SELECT /* {marker} */ g, md5(g::text) FROM (SELECT generate_series(1::bigint, 300000000) AS g) s"
        ),
        Engine::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) SELECT /* {marker} */ x, hex(x) FROM c"
        ),
        _ => format!("SELECT /* {marker} */ a.id, b.id FROM big_table a CROSS JOIN big_table b"),
    };
    let token = CancellationToken::new();
    let rec = recorder();
    let job = {
        let (driver, token, cb, path) = (
            c.driver.clone(),
            token.clone(),
            rec.callback.clone(),
            path.clone(),
        );
        let db = c.database.clone();
        tokio::spawn(async move {
            export(
                driver,
                ExportSource::Query {
                    database: Some(db),
                    sql,
                },
                csv_format(',', true, ""),
                path,
                cb,
                token,
            )
            .await
        })
    };
    eventually(
        Duration::from_secs(20),
        "the export writes its first rows",
        || async { rec.all().iter().any(|p| p.rows > 0) },
    )
    .await;
    let started = Instant::now();
    token.cancel();
    let done = job.await.unwrap().unwrap();
    assert!(
        done.cancelled && done.done && done.error.is_none(),
        "{done:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        started.elapsed()
    );
    assert!(done.rows > 0, "some rows were written before the cancel");
    // What was written is a valid prefix of the file.
    assert!(path.exists());
    assert_eq!(done.bytes, std::fs::metadata(&path).unwrap().len());
    // The server stopped producing rows.
    if !c.is_sqlite() {
        eventually(Duration::from_secs(5), "the server stops the export query", || async {
            let sql = if c.is_pg() {
                format!("SELECT count(*)::text FROM pg_stat_activity WHERE state = 'active' AND query LIKE '%{marker}%' AND pid <> pg_backend_pid()")
            } else {
                format!("SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.processlist WHERE command = 'Query' AND info LIKE '%{marker}%' AND id <> CONNECTION_ID()")
            };
            c.side_text(&sql).await.as_deref() == Some("0")
        })
        .await;
    }
    // Cancelled before it even starts.
    let token = CancellationToken::new();
    token.cancel();
    let done = export(
        c.driver.clone(),
        table_source(c, "irrelevant"),
        csv_format(',', true, ""),
        dir.file("never.csv"),
        recorder().callback,
        token,
    )
    .await
    .unwrap();
    assert!(done.cancelled && done.rows == 0);
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(cancelling_an_export_stops_it_quickly, sql);

async fn the_seeded_million_row_table_exports_completely(c: &Ctx) {
    let dir = TempDir::new();
    let path = dir.file("big.jsonl");
    let started = Instant::now();
    let done = export_to(
        c,
        ExportSource::Table {
            request: {
                let mut r = c.browse("big_table");
                r.columns = vec!["id".into()];
                r
            },
        },
        ExportFormat::JsonLines,
        &path,
    )
    .await
    .unwrap();
    assert_eq!(done.rows, 1_000_000);
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1_000_000);
    assert!(text.lines().any(|l| l == "{\"id\":1000000}"));
}
each_engine!(
    the_seeded_million_row_table_exports_completely,
    postgres,
    mysql,
    mariadb
);

// ---------------------------------------------------------------------------
// CSV import
// ---------------------------------------------------------------------------

fn import_spec(c: &Ctx, table: &str, path: &Path, mapping: &[Option<&str>]) -> CsvImport {
    CsvImport {
        path: path.to_string_lossy().into_owned(),
        table: c.tref(table),
        delimiter: ',',
        has_header: true,
        mapping: mapping.iter().map(|m| m.map(str::to_owned)).collect(),
        null_as: None,
    }
}

async fn run_import(c: &Ctx, spec: CsvImport) -> (dbcore::Result<JobProgress>, Vec<JobProgress>) {
    let rec = recorder();
    let result = import_csv(
        c.driver.clone(),
        spec,
        rec.callback.clone(),
        CancellationToken::new(),
    )
    .await;
    (result, rec.all())
}

/// `people(id PK, name NOT NULL, note, qty)`.
async fn people_table(c: &Ctx) -> String {
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(500)"),
    };
    let t = c.scratch("people");
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY, name {text} NOT NULL, note {text}, qty {int})",
        c.qt(&t)
    ))
    .await;
    t
}

async fn csv_import_quoting_mapping_and_nulls(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("people.csv");
    // CSV columns: qty, id, ignored, name, note. Quoted commas, quotes, newlines, empty fields, NULL markers.
    let content = "qty,id,ignored,name,note\r\n\
        10,1,x,Ada,\"likes, commas\"\r\n\
        20,2,x,\"Bob \"\"the builder\"\"\",\"line one\r\nline two\"\r\n\
        30,3,x,Ünï 日本語 🎉,NULL\r\n\
        40,4,x,  spaced  ,\r\n\
        50,5,x,Eve\r\n\
        60,6,x,Frank,extra,fields,ignored\r\n";
    std::fs::write(&path, content).unwrap();
    let mut spec = import_spec(
        c,
        &t,
        &path,
        &[Some("qty"), Some("id"), None, Some("name"), Some("note")],
    );
    spec.null_as = Some("NULL".into());
    let (result, reports) = run_import(c, spec).await;
    let done = result.unwrap();
    assert!(done.done && !done.cancelled && done.error.is_none());
    assert_eq!(done.rows, 6);
    assert!(reports.last().unwrap().done);

    let rows = c
        .rows(&format!(
            "SELECT id, name, note, qty FROM {} ORDER BY id",
            c.qt(&t)
        ))
        .await;
    let text = |s: &str| Value::Text(s.to_owned());
    assert_eq!(
        rows[0],
        vec![
            Value::Int(1),
            text("Ada"),
            text("likes, commas"),
            Value::Int(10)
        ]
    );
    assert_eq!(rows[1][1], text("Bob \"the builder\""));
    assert_eq!(rows[1][2], text("line one\r\nline two"));
    assert_eq!(rows[2][1], text("Ünï 日本語 🎉"));
    assert_eq!(rows[2][2], Value::Null, "the NULL marker becomes NULL");
    assert_eq!(rows[3][1], text("  spaced  "), "spaces are kept");
    assert_eq!(
        rows[3][2],
        text(""),
        "an empty field is an empty string unless it is the NULL marker"
    );
}
each_engine!(csv_import_quoting_mapping_and_nulls, sql);

async fn csv_import_empty_fields_as_null(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("people.csv");
    std::fs::write(
        &path,
        "id;name;note;qty\n1;Ada;;5\n2;Bob;\"semi;colon\";\n3;Cy;;\n4;Short\n",
    )
    .unwrap();
    let mut spec = import_spec(
        c,
        &t,
        &path,
        &[Some("id"), Some("name"), Some("note"), Some("qty")],
    );
    spec.delimiter = ';';
    spec.null_as = Some(String::new());
    let (result, _) = run_import(c, spec).await;
    assert_eq!(result.unwrap().rows, 4);
    let rows = c
        .rows(&format!(
            "SELECT id, name, note, qty FROM {} ORDER BY id",
            c.qt(&t)
        ))
        .await;
    let text = |s: &str| Value::Text(s.to_owned());
    assert_eq!(
        rows[0],
        vec![Value::Int(1), text("Ada"), Value::Null, Value::Int(5)]
    );
    assert_eq!(
        rows[1],
        vec![Value::Int(2), text("Bob"), text("semi;colon"), Value::Null]
    );
    assert_eq!(
        rows[2],
        vec![Value::Int(3), text("Cy"), Value::Null, Value::Null]
    );
    assert_eq!(
        rows[3],
        vec![Value::Int(4), text("Short"), Value::Null, Value::Null],
        "a short row leaves the rest NULL"
    );

    // Tab and pipe delimiters, no header row.
    for (delimiter, sep) in [('\t', "\t"), ('|', "|")] {
        c.side_exec(&format!("DELETE FROM {}", c.qt(&t))).await;
        let path = dir.file("nohead.csv");
        std::fs::write(&path, format!("1{sep}A{sep}x\n2{sep}B{sep}y\n")).unwrap();
        let mut spec = import_spec(c, &t, &path, &[Some("id"), Some("name"), Some("note")]);
        spec.delimiter = delimiter;
        spec.has_header = false;
        let (result, _) = run_import(c, spec).await;
        assert_eq!(result.unwrap().rows, 2, "{delimiter:?}");
        assert_eq!(c.side_count(&t).await, 2);
    }
}
each_engine!(csv_import_empty_fields_as_null, sql);

async fn csv_import_bad_rows_roll_back_their_batch(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("bad.csv");
    // 2500 rows; row 2200 has an empty name, which the NOT NULL column refuses.
    let mut content = String::from("id,name\n");
    for i in 1..=2500 {
        if i == 2200 {
            content.push_str(&format!("{i},\n"));
        } else {
            content.push_str(&format!("{i},person {i}\n"));
        }
    }
    std::fs::write(&path, content).unwrap();
    let mut spec = import_spec(c, &t, &path, &[Some("id"), Some("name")]);
    spec.null_as = Some(String::new());
    let (result, reports) = run_import(c, spec).await;
    let err = result.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    // The first two batches of 1000 are committed; the third rolled back as a whole.
    assert_eq!(c.side_count(&t).await, 2000);
    let last = reports.last().unwrap();
    assert!(last.done && last.rows == 2000);
    let message = last.error.as_deref().unwrap();
    assert!(
        message.contains("2000 rows were already imported"),
        "{message}"
    );
    assert_eq!(
        c.scalar(&format!("SELECT MAX(id) FROM {}", c.qt(&t))).await,
        Value::Int(2000)
    );

    // A duplicate key within the file stops the import the same way.
    c.side_exec(&format!("DELETE FROM {}", c.qt(&t))).await;
    std::fs::write(&path, "id,name\n1,a\n2,b\n1,again\n").unwrap();
    let (result, _) = run_import(c, import_spec(c, &t, &path, &[Some("id"), Some("name")])).await;
    assert!(matches!(result, Err(Error::Database { .. })));
    assert_eq!(c.side_count(&t).await, 0, "all three rows were one batch");
}
each_engine!(csv_import_bad_rows_roll_back_their_batch, sql);

async fn csv_import_type_errors_on_strict_engines(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("types.csv");
    std::fs::write(&path, "id,name,qty\n1,ok,5\n2,bad,not a number\n").unwrap();
    let (result, reports) = run_import(
        c,
        import_spec(c, &t, &path, &[Some("id"), Some("name"), Some("qty")]),
    )
    .await;
    let err = result.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    assert_eq!(c.side_count(&t).await, 0);
    assert!(reports.last().unwrap().error.is_some());
}
each_engine!(
    csv_import_type_errors_on_strict_engines,
    postgres,
    mysql,
    mariadb
);

async fn csv_import_invalid_requests(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("ok.csv");
    std::fs::write(&path, "id,name\n1,a\n").unwrap();
    // Nothing mapped.
    let (result, reports) = run_import(c, import_spec(c, &t, &path, &[None, None])).await;
    assert!(matches!(result, Err(Error::InvalidInput(_))));
    assert!(reports.last().unwrap().error.is_some());
    // Bad delimiter.
    let mut spec = import_spec(c, &t, &path, &[Some("id"), Some("name")]);
    spec.delimiter = '"';
    assert!(matches!(
        run_import(c, spec).await.0,
        Err(Error::InvalidInput(_))
    ));
    // Missing file.
    let spec = import_spec(c, &t, &dir.file("missing.csv"), &[Some("id"), Some("name")]);
    assert!(matches!(run_import(c, spec).await.0, Err(Error::Io(_))));
    // Unknown target column or table.
    let spec = import_spec(c, &t, &path, &[Some("id"), Some("no_such_column")]);
    assert!(matches!(
        run_import(c, spec).await.0,
        Err(Error::Database { .. })
    ));
    let spec = import_spec(c, "no_such_table_xyz", &path, &[Some("id"), Some("name")]);
    assert!(run_import(c, spec).await.0.is_err());
    assert_eq!(c.side_count(&t).await, 0);
    // Empty file, header-only file.
    for body in ["", "id,name\n"] {
        std::fs::write(&path, body).unwrap();
        let (result, _) =
            run_import(c, import_spec(c, &t, &path, &[Some("id"), Some("name")])).await;
        let done = result.unwrap();
        assert!(done.done && done.rows == 0, "{body:?}: {done:?}");
    }
    assert_eq!(c.side_count(&t).await, 0);
}
each_engine!(csv_import_invalid_requests, sql);

async fn csv_import_large_files_and_large_values(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("many.csv");
    let mut content = String::from("id,name,note\n");
    for i in 1..=12_345 {
        content.push_str(&format!("{i},name {i},\"note, {i}\"\n"));
    }
    std::fs::write(&path, content).unwrap();
    let started = Instant::now();
    let (result, reports) = run_import(
        c,
        import_spec(c, &t, &path, &[Some("id"), Some("name"), Some("note")]),
    )
    .await;
    let done = result.unwrap();
    assert_eq!(done.rows, 12_345);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(c.side_count(&t).await, 12_345);
    assert_eq!(
        c.rows(&format!(
            "SELECT name, note FROM {} WHERE id = 12345",
            c.qt(&t)
        ))
        .await,
        vec![vec![
            Value::Text("name 12345".into()),
            Value::Text("note, 12345".into())
        ]]
    );
    for pair in reports.windows(2) {
        assert!(pair[0].rows <= pair[1].rows);
    }
    assert_eq!(
        done.bytes,
        std::fs::metadata(&path).unwrap().len(),
        "bytes count the file position"
    );

    // Values big enough that one INSERT cannot hold them all.
    let (text_ty, int) = match c.engine {
        Engine::Postgres => ("text", "integer"),
        Engine::Sqlite => ("TEXT", "INTEGER"),
        _ => ("LONGTEXT", "INT"),
    };
    let big = c.scratch("big_values");
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY, v {text_ty})",
        c.qt(&big)
    ))
    .await;
    let path = dir.file("big.csv");
    let mut content = String::from("id,v\n");
    for i in 1..=5 {
        let value = format!("{i}-{}", "x".repeat(400 * 1024));
        content.push_str(&format!("{i},\"{value}\"\n"));
    }
    std::fs::write(&path, content).unwrap();
    let (result, _) = run_import(c, import_spec(c, &big, &path, &[Some("id"), Some("v")])).await;
    assert_eq!(result.unwrap().rows, 5);
    assert_eq!(
        c.scalar(&format!(
            "SELECT LENGTH(v) FROM {} WHERE id = 3",
            c.qt(&big)
        ))
        .await,
        Value::Int(2 + 400 * 1024)
    );
}
each_engine!(csv_import_large_files_and_large_values, sql);

async fn cancelling_an_import_keeps_committed_batches(c: &Ctx) {
    let t = people_table(c).await;
    let dir = TempDir::new();
    let path = dir.file("long.csv");
    let mut content = String::from("id,name\n");
    for i in 1..=60_000 {
        content.push_str(&format!("{i},person {i}\n"));
    }
    std::fs::write(&path, content).unwrap();
    let token = CancellationToken::new();
    let rec = recorder();
    let job = {
        let spec = import_spec(c, &t, &path, &[Some("id"), Some("name")]);
        let (driver, token, cb) = (c.driver.clone(), token.clone(), rec.callback.clone());
        tokio::spawn(async move { import_csv(driver, spec, cb, token).await })
    };
    eventually(
        Duration::from_secs(20),
        "some rows are imported",
        || async { c.side_count(&t).await >= 1000 },
    )
    .await;
    token.cancel();
    let done = job.await.unwrap().unwrap();
    assert!(
        done.cancelled && done.done && done.error.is_none(),
        "{done:?}"
    );
    let stored = c.side_count(&t).await;
    assert!((1000..60_000).contains(&stored), "{stored}");
    assert_eq!(stored % 1000, 0, "only whole batches are committed");
    // A batch that commits just as the cancel lands is stored but not counted.
    assert!(
        done.rows as i64 <= stored && stored - done.rows as i64 <= 1000,
        "{done:?} vs {stored}"
    );
}
each_engine!(cancelling_an_import_keeps_committed_batches, sql);

async fn export_then_import_round_trip(c: &Ctx) {
    // Everything except binary data (CSV has no byte syntax every database reads).
    let src = export_fixture(c, "rt_src").await;
    let dst = c.scratch("rt_dst");
    c.side_exec(&export_ddl(c, &dst)).await;
    let dir = TempDir::new();
    let path = dir.file("rt.csv");
    let mut req = c.browse(&src);
    req.columns = COLUMNS
        .iter()
        .filter(|n| **n != "raw")
        .map(|n| (*n).to_owned())
        .collect();
    export_to(
        c,
        ExportSource::Table {
            request: req.clone(),
        },
        csv_format(',', true, "\\N"),
        &path,
    )
    .await
    .unwrap();
    let mapping: Vec<Option<&str>> = COLUMNS
        .iter()
        .filter(|n| **n != "raw")
        .map(|n| Some(*n))
        .collect();
    let mut spec = import_spec(c, &dst, &path, &mapping);
    spec.null_as = Some("\\N".into());
    let (result, _) = run_import(c, spec).await;
    assert_eq!(result.unwrap().rows, 10);
    let mut dst_req = c.browse(&dst);
    dst_req.columns = req.columns.clone();
    let mut a = c.all_rows(&req).await;
    let mut b = c.all_rows(&dst_req).await;
    a.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    b.sort_by_key(|r| ids(std::slice::from_ref(r), 0)[0]);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y, "a CSV export imports back to the same values");
    }
}
each_engine!(export_then_import_round_trip, sql);

// ---------------------------------------------------------------------------
// CSV preview
// ---------------------------------------------------------------------------

#[test]
fn csv_preview_sniffs_delimiters_and_reads_headers() {
    let dir = TempDir::new();
    let write = |name: &str, body: &str| {
        let path = dir.file(name);
        std::fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    };
    let cases = [
        ("comma.csv", "a,b,c\n1,2,3\n4,5,6\n", ','),
        ("semi.csv", "a;b;c\n1;2;3\n4;5;6\n", ';'),
        ("tab.csv", "a\tb\tc\n1\t2\t3\n4\t5\t6\n", '\t'),
        ("pipe.csv", "a|b|c\n1|2|3\n4|5|6\n", '|'),
        ("quoted.csv", "a;b\n\"x,y,z\";2\n\"p,q,r\";4\n", ';'),
    ];
    for (name, body, delimiter) in cases {
        let preview = csv_preview(&write(name, body), None, 10).unwrap();
        assert_eq!(preview.delimiter, delimiter, "{name}");
        assert_eq!(preview.rows.len(), 2, "{name}");
        assert_eq!(preview.headers[0], "a");
    }
    // A BOM does not end up in the first header; row limits apply; quoted newlines stay inside.
    let path = write("bom.csv", "\u{feff}id,note\n1,\"two\nlines\"\n2,b\n3,c\n");
    let preview = csv_preview(&path, Some(','), 2).unwrap();
    assert_eq!(preview.headers, ["id", "note"]);
    assert_eq!(preview.rows, vec![vec!["1", "two\nlines"], vec!["2", "b"]]);
    // Empty file, header only, ragged rows.
    let preview = csv_preview(&write("empty.csv", ""), None, 5).unwrap();
    assert!(preview.headers.is_empty() && preview.rows.is_empty());
    let preview = csv_preview(&write("head.csv", "a,b\n"), None, 5).unwrap();
    assert_eq!(preview.headers, ["a", "b"]);
    assert!(preview.rows.is_empty());
    let preview = csv_preview(&write("ragged.csv", "a,b,c\n1\n1,2,3,4\n"), Some(','), 5).unwrap();
    assert_eq!(preview.rows, vec![vec!["1"], vec!["1", "2", "3", "4"]]);
    // Errors.
    assert!(matches!(
        csv_preview(&dir.file("nope.csv").to_string_lossy(), None, 5),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        csv_preview(&path, Some('"'), 5),
        Err(Error::InvalidInput(_))
    ));
}

// ---------------------------------------------------------------------------
// MongoDB
// ---------------------------------------------------------------------------

mod mongo_files {
    use super::*;
    use mongodb::bson::{self, Bson, doc, oid::ObjectId};

    async fn export_collection(c: &Ctx, suffix: &str) -> String {
        let name = c.scratch(suffix);
        let oid = ObjectId::parse_str("65f000000000000000000002").unwrap();
        c.mongo_coll(&name)
            .insert_many([
                doc! {
                    "_id": 1, "name": "plain", "n": 5, "big": 9_007_199_254_740_993i64,
                    "amount": "1.50".parse::<bson::Decimal128>().unwrap(), "ok": true,
                    "at": bson::DateTime::from_millis(1_709_213_820_123),
                    "doc": { "a": [1, 2], "z": { "k": null } }, "arr": [1, "x"],
                    "raw": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![0xde, 0xad] },
                },
                doc! { "_id": 2, "name": "with,comma \"q\"\nnewline", "n": Bson::Null },
                doc! { "_id": oid, "name": "" },
                doc! { "_id": "str-id", "name": "ünï 🎉" },
            ])
            .await
            .unwrap();
        name
    }

    async fn mongo_exports_keep_documents_intact(c: &Ctx) {
        let t = export_collection(c, "exp").await;
        let dir = TempDir::new();
        let columns = [
            "_id", "name", "n", "big", "amount", "ok", "at", "doc", "arr", "raw",
        ];

        // JSON: nested values nest; ids and dates are text; absent fields are null.
        let path = dir.file("out.json");
        let done = export_to(
            c,
            table_source(c, &t),
            ExportFormat::Json { pretty: true },
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 4);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let docs = parsed.as_array().unwrap();
        assert_eq!(docs.len(), 4);
        let first = docs[0].as_object().unwrap();
        assert_eq!(
            first.keys().map(String::as_str).collect::<Vec<_>>(),
            columns
        );
        assert_eq!(first["_id"], 1);
        assert_eq!(
            first["big"], "9007199254740993",
            "int64 beyond 2^53 is a string"
        );
        assert_eq!(first["amount"], "1.50");
        assert_eq!(first["ok"], true);
        assert_eq!(first["at"], "2024-02-29T13:37:00.123Z");
        assert_eq!(
            first["doc"],
            serde_json::json!({ "a": [1, 2], "z": { "k": null } })
        );
        assert_eq!(first["arr"], serde_json::json!([1, "x"]));
        assert_eq!(first["raw"], "3q0=");
        // An explicit null and a missing field both come out as null, not as an empty string.
        assert_eq!(docs[1]["n"], serde_json::Value::Null);
        assert_eq!(
            docs[1]["big"],
            serde_json::Value::Null,
            "absent fields export as null"
        );
        assert_eq!(docs[1]["doc"], serde_json::Value::Null);
        assert_eq!(docs[1]["name"], "with,comma \"q\"\nnewline");
        // (BSON orders numbers, then strings, then ObjectIds.)
        assert_eq!(docs[2]["_id"], "str-id");
        assert_eq!(docs[2]["name"], "ünï 🎉");
        assert_eq!(docs[3]["_id"], "65f000000000000000000002");
        assert_eq!(docs[3]["name"], "");

        // JSON lines: the same, one per line.
        let path = dir.file("out.jsonl");
        export_to(c, table_source(c, &t), ExportFormat::JsonLines, &path)
            .await
            .unwrap();
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1]["big"], serde_json::Value::Null);

        // CSV: nested documents are JSON text; null and absent use the marker, '' stays empty.
        let path = dir.file("out.csv");
        export_to(c, table_source(c, &t), csv_format(',', true, "\\N"), &path)
            .await
            .unwrap();
        let records = read_csv(&path, b',');
        assert_eq!(records[0], columns);
        assert_eq!(records[1][0], "1");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&records[1][7]).unwrap(),
            serde_json::json!({ "a": [1, 2], "z": { "k": null } })
        );
        assert_eq!(records[2][1], "with,comma \"q\"\nnewline");
        assert_eq!(records[2][2], "\\N", "explicit null");
        assert_eq!(records[2][3], "\\N", "absent field");
        assert_eq!(records[3][0], "str-id");
        assert_eq!(records[3][1], "ünï 🎉");
        assert_eq!(records[4][0], "65f000000000000000000002");
        assert_eq!(records[4][1], "", "an empty string is not null");

        // Filters and columns apply; SQL is not offered for Mongo.
        let mut req = c.browse(&t);
        req.filters = vec![filter("n", FilterOp::Eq, Some("5"))];
        req.columns = vec!["name".into()];
        let path = dir.file("subset.jsonl");
        let done = export_to(
            c,
            ExportSource::Table { request: req },
            ExportFormat::JsonLines,
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            r#"{"_id":1,"name":"plain"}"#
        );
        let err = export_to(
            c,
            table_source(c, &t),
            ExportFormat::Sql {
                table: "t".into(),
                rows_per_insert: 10,
            },
            &dir.file("x.sql"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err:?}");

        // A shell query is an export source too.
        let path = dir.file("query.csv");
        let done = export_to(
            c,
            ExportSource::Query { database: None, sql: format!("db.{t}.find({{ _id: {{ $in: [1, 2] }} }}, {{ name: 1, n: 1 }}).sort({{ _id: 1 }})") },
            csv_format(',', true, ""),
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 2);
        let records = read_csv(&path, b',');
        assert_eq!(records[0], ["_id", "name", "n"]);
        assert_eq!(records[1], ["1", "plain", "5"]);
    }
    each_engine!(mongo_exports_keep_documents_intact, mongo);

    async fn mongo_exports_big_and_cancellable(c: &Ctx) {
        let dir = TempDir::new();
        // 20,000 documents, complete and in order.
        let t = c.scratch("exp_many");
        let ids: Vec<i32> = (1..=20_000).collect();
        for chunk in ids.chunks(5000) {
            c.mongo_coll(&t)
                .insert_many(
                    chunk
                        .iter()
                        .map(|i| doc! { "_id": *i, "v": format!("v{i}") }),
                )
                .await
                .unwrap();
        }
        let path = dir.file("many.jsonl");
        let done = export_to(c, table_source(c, &t), ExportFormat::JsonLines, &path)
            .await
            .unwrap();
        assert_eq!(done.rows, 20_000);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 20_000);
        assert_eq!(
            text.lines().last().unwrap(),
            r#"{"_id":20000,"v":"v20000"}"#
        );

        // The million-document collection exports completely.
        let started = Instant::now();
        let path = dir.file("big.jsonl");
        let mut req = c.browse("big_collection");
        req.columns = vec!["name".into()];
        let done = export_to(
            c,
            ExportSource::Table { request: req },
            ExportFormat::JsonLines,
            &path,
        )
        .await
        .unwrap();
        assert_eq!(done.rows, 1_000_000);
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "{:?}",
            started.elapsed()
        );
        std::fs::remove_file(&path).unwrap();

        // Cancelling stops it quickly.
        let token = CancellationToken::new();
        let rec = recorder();
        let job = {
            let (driver, token, cb, path) = (
                c.driver.clone(),
                token.clone(),
                rec.callback.clone(),
                dir.file("cancelled.csv"),
            );
            let source = table_source(c, "big_collection");
            tokio::spawn(async move {
                export(driver, source, csv_format(',', true, ""), path, cb, token).await
            })
        };
        eventually(
            Duration::from_secs(20),
            "rows are being written",
            || async { rec.all().iter().any(|p| p.rows > 0) },
        )
        .await;
        let cancelled_at = Instant::now();
        token.cancel();
        let done = job.await.unwrap().unwrap();
        assert!(done.cancelled && done.done, "{done:?}");
        assert!(cancelled_at.elapsed() < Duration::from_secs(5));
        assert!(done.rows < 1_000_000);
    }
    each_engine!(mongo_exports_big_and_cancellable, mongo);

    fn mongo_import(c: &Ctx, table: &str, path: &Path, mapping: &[Option<&str>]) -> CsvImport {
        import_spec(c, table, path, mapping)
    }

    async fn mongo_import_types_values_by_the_existing_documents(c: &Ctx) {
        let t = c.scratch("imp");
        let oid = ObjectId::parse_str("65f000000000000000000001").unwrap();
        c.mongo_coll(&t)
            .insert_one(doc! {
                "_id": 0, "name": "seed", "n": 1, "big": 1i64, "price": 1.5, "ok": true,
                "at": bson::DateTime::from_millis(0), "ref": oid, "obj": { "a": 1 }, "tags": ["x"],
                "dec": "1.0".parse::<bson::Decimal128>().unwrap(),
            })
            .await
            .unwrap();
        let dir = TempDir::new();
        let path = dir.file("docs.csv");
        std::fs::write(
            &path,
            "_id,name,n,big,price,ok,at,ref,obj,tags,dec,extra\n\
             1,Ada,10,9007199254740993,2.5,true,2024-02-29 13:14:15,65f000000000000000000002,\"{\"\"a\"\": [1, 2]}\",\"[\"\"p\"\", \"\"q\"\"]\",19.99,free text\n\
             2,\"quoted, \"\"name\"\"\",,,,,,,,,,\n\
             3,NULL,not a number,,1e3,0,2024-02-29,,,,,\n\
             4,\"\",7,,,false,,,,,,\n",
        )
        .unwrap();
        let mapping: Vec<Option<&str>> = [
            "_id", "name", "n", "big", "price", "ok", "at", "ref", "obj", "tags", "dec", "extra",
        ]
        .into_iter()
        .map(Some)
        .collect();
        let mut spec = mongo_import(c, &t, &path, &mapping);
        spec.null_as = Some("NULL".into());
        let (result, _) = run_import(c, spec).await;
        assert_eq!(result.unwrap().rows, 4);
        let get = |id: i32| {
            let coll = c.mongo_coll(&t);
            async move { coll.find_one(doc! { "_id": id }).await.unwrap().unwrap() }
        };
        let d = get(1).await;
        assert_eq!(d.get("name"), Some(&Bson::String("Ada".into())));
        assert_eq!(d.get("n"), Some(&Bson::Int32(10)));
        assert_eq!(d.get("big"), Some(&Bson::Int64(9_007_199_254_740_993)));
        assert_eq!(d.get("price"), Some(&Bson::Double(2.5)));
        assert_eq!(d.get("ok"), Some(&Bson::Boolean(true)));
        assert_eq!(
            d.get("at"),
            Some(&Bson::DateTime(bson::DateTime::from_millis(
                1_709_212_455_000
            )))
        );
        assert_eq!(
            d.get("ref"),
            Some(&Bson::ObjectId(
                ObjectId::parse_str("65f000000000000000000002").unwrap()
            ))
        );
        assert_eq!(
            d.get_document("obj").unwrap().get_array("a").unwrap().len(),
            2
        );
        assert_eq!(
            d.get_array("tags").unwrap(),
            &vec![Bson::String("p".into()), Bson::String("q".into())]
        );
        assert_eq!(
            d.get("dec"),
            Some(&Bson::Decimal128("19.99".parse().unwrap()))
        );
        assert_eq!(
            d.get("extra"),
            Some(&Bson::String("free text".into())),
            "unknown columns stay text"
        );
        // Blank cells of non-text columns leave the field out; text columns keep ''.
        let d = get(2).await;
        assert_eq!(
            d.get("name"),
            Some(&Bson::String("quoted, \"name\"".into()))
        );
        for field in ["n", "big", "price", "ok", "at", "ref", "obj", "tags", "dec"] {
            assert!(!d.contains_key(field), "{field} should be absent: {d:?}");
        }
        assert_eq!(d.get("extra"), Some(&Bson::String(String::new())));
        // The NULL marker leaves the field out; text that is not a number stays text.
        let d = get(3).await;
        assert!(!d.contains_key("name"));
        assert_eq!(d.get("n"), Some(&Bson::String("not a number".into())));
        assert_eq!(d.get("price"), Some(&Bson::Double(1000.0)));
        assert_eq!(d.get("ok"), Some(&Bson::Boolean(false)));
        assert_eq!(
            d.get("at"),
            Some(&Bson::DateTime(bson::DateTime::from_millis(
                1_709_164_800_000
            )))
        );
        assert_eq!(get(4).await.get("name"), Some(&Bson::String(String::new())));

        // Into an empty collection everything is text.
        let fresh = c.scratch("imp_fresh");
        c.mongo_db().create_collection(&fresh).await.unwrap();
        let path = dir.file("fresh.csv");
        std::fs::write(&path, "_id,n,flag\n1,5,true\n").unwrap();
        let (result, _) = run_import(
            c,
            mongo_import(c, &fresh, &path, &[Some("_id"), Some("n"), Some("flag")]),
        )
        .await;
        assert_eq!(result.unwrap().rows, 1);
        let d = c
            .mongo_coll(&fresh)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.get("n"), Some(&Bson::String("5".into())));
        assert_eq!(d.get("_id"), Some(&Bson::String("1".into())));
    }
    each_engine!(mongo_import_types_values_by_the_existing_documents, mongo);

    async fn mongo_import_failures_keep_earlier_documents(c: &Ctx) {
        let t = c.scratch("imp_fail");
        c.mongo_coll(&t)
            .insert_one(doc! { "_id": 0, "n": 0 })
            .await
            .unwrap();
        let dir = TempDir::new();
        let path = dir.file("dup.csv");
        let mut content = String::from("_id,n\n");
        for i in 1..=2500 {
            // Row 2200 repeats an earlier _id.
            content.push_str(&format!("{},{i}\n", if i == 2200 { 5 } else { i }));
        }
        std::fs::write(&path, content).unwrap();
        let (result, reports) =
            run_import(c, mongo_import(c, &t, &path, &[Some("_id"), Some("n")])).await;
        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::Database { code: Some(code), .. } if code == "11000"),
            "{err:?}"
        );
        // No transactions here: the batches before the failure and the rows before the
        // duplicate in its own batch stay.
        let stored = c.side_count(&t).await;
        assert_eq!(stored, 1 + 2199, "{stored}");
        assert!(reports.last().unwrap().error.is_some());
        // Nothing mapped, missing file.
        let (result, _) = run_import(c, mongo_import(c, &t, &path, &[None, None])).await;
        assert!(matches!(result, Err(Error::InvalidInput(_))));
        let (result, _) = run_import(
            c,
            mongo_import(c, &t, &dir.file("missing.csv"), &[Some("_id")]),
        )
        .await;
        assert!(matches!(result, Err(Error::Io(_))));
        // A view is read-only.
        let view = c.scratch("imp_view");
        c.mongo_db()
            .run_command(doc! { "create": &view, "viewOn": &t, "pipeline": [] })
            .await
            .unwrap();
        std::fs::write(&path, "_id,n\n9001,1\n").unwrap();
        let (result, _) =
            run_import(c, mongo_import(c, &view, &path, &[Some("_id"), Some("n")])).await;
        assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    }
    each_engine!(mongo_import_failures_keep_earlier_documents, mongo);

    async fn mongo_export_then_import_round_trip(c: &Ctx) {
        let src = export_collection(c, "rt_src").await;
        let dst = c.scratch("rt_dst");
        // Seed the destination so its column types are known.
        c.mongo_coll(&dst)
            .insert_one(doc! { "_id": 0, "name": "", "n": 0, "big": 0i64, "amount": "0".parse::<bson::Decimal128>().unwrap(), "ok": false,
                               "at": bson::DateTime::from_millis(0), "doc": { "a": 1 }, "arr": [0] })
            .await
            .unwrap();
        let dir = TempDir::new();
        let path = dir.file("rt.csv");
        let mut req = c.browse(&src);
        req.filters = vec![filter("_id", FilterOp::Eq, Some("1"))];
        req.columns = ["name", "n", "big", "amount", "ok", "at", "doc", "arr"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        export_to(
            c,
            ExportSource::Table { request: req },
            csv_format(',', true, "\\N"),
            &path,
        )
        .await
        .unwrap();
        let mapping: Vec<Option<&str>> = [
            "_id", "name", "n", "big", "amount", "ok", "at", "doc", "arr",
        ]
        .into_iter()
        .map(Some)
        .collect();
        let mut spec = mongo_import(c, &dst, &path, &mapping);
        spec.null_as = Some("\\N".into());
        let (result, _) = run_import(c, spec).await;
        assert_eq!(result.unwrap().rows, 1);
        let a = c
            .mongo_coll(&src)
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .unwrap();
        let mut b = c
            .mongo_coll(&dst)
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .unwrap();
        b.remove("raw");
        for field in ["name", "n", "big", "amount", "ok", "at", "doc", "arr"] {
            assert_eq!(
                a.get(field),
                b.get(field),
                "{field} survives export -> import"
            );
        }
    }
    each_engine!(mongo_export_then_import_round_trip, mongo);
}
