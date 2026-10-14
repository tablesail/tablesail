//! Smoke tests against every engine's "nasty schema" (see testdb/ and docker-compose.yml).
//!
//! SQLite runs in memory and always executes. The server engines connect to the
//! docker-compose databases; when one is unreachable the test is skipped with a
//! message, unless TABLESAIL_REQUIRE_DBS=1 (set in CI), in which case it fails.
//!
//! URLs can be overridden with TABLESAIL_{POSTGRES,MYSQL,MARIADB,MONGO}_URL.

use mongodb::bson::doc;
use sqlx::{Connection, MySqlConnection, PgConnection, SqliteConnection};

const BIG_TABLE_ROWS: i64 = 1_000_000;

fn url(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_owned())
}

/// Handles an unreachable server: skip locally, fail in CI.
fn unavailable(engine: &str, err: impl std::fmt::Display) {
    if std::env::var("TABLESAIL_REQUIRE_DBS").is_ok_and(|v| v == "1") {
        panic!("{engine} is required but unreachable: {err}");
    }
    eprintln!("SKIPPED {engine}: {err} (run `docker compose up -d --wait`)");
}

fn assert_has_all(engine: &str, found: &[String], expected: &[&str]) {
    for name in expected {
        assert!(
            found.iter().any(|f| f == name),
            "{engine}: expected `{name}` in {found:?}"
        );
    }
}

/// Tables every SQL engine's nasty schema must contain.
const SQL_TABLES: &[&str] = &[
    "all_types",
    "big_table",
    "no_pk",
    "composite_pk",
    "select",
    "weird \"quoted\" name; DROP TABLE x;--",
    "customers",
    "orders",
    "computed",
    "customer_totals",
    "empty_table",
];

#[tokio::test]
async fn sqlite_nasty_schema() {
    let mut conn = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../../../testdb/sqlite/nasty.sql"))
        .execute(&mut conn)
        .await
        .expect("seed sqlite");

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_has_all("sqlite", &tables, SQL_TABLES);
    assert_has_all(
        "sqlite",
        &tables,
        &["strict_table", "without_rowid", "docs_fts"],
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM big_table")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(rows, BIG_TABLE_ROWS);

    // Dynamic typing: one column, several storage classes.
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT typeof(c_integer) FROM all_types ORDER BY 1")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_eq!(kinds, ["integer", "null", "text"]);
}

#[tokio::test]
async fn postgres_nasty_schema() {
    let url = url(
        "TABLESAIL_POSTGRES_URL",
        "postgres://tablesail:tablesail@127.0.0.1:25432/tablesail",
    );
    let mut conn = match PgConnection::connect(&url).await {
        Ok(conn) => conn,
        Err(err) => return unavailable("postgres", err),
    };

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_has_all("postgres", &tables, SQL_TABLES);
    assert_has_all(
        "postgres",
        &tables,
        &["user", "MixedCase", "wide_table", "events"],
    );

    let schemas: Vec<String> =
        sqlx::query_scalar("SELECT schema_name::text FROM information_schema.schemata")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_has_all(
        "postgres",
        &schemas,
        &["billing", "Mixed Case Schema", "empty_schema"],
    );

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM big_table")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(rows, BIG_TABLE_ROWS);

    let no_pk_constraints: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.table_constraints
         WHERE table_name = 'no_pk' AND constraint_type = 'PRIMARY KEY'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(no_pk_constraints, 0);

    // bigint beyond JavaScript's safe integer range survives as text.
    let big: String = sqlx::query_scalar("SELECT c_bigint::text FROM all_types WHERE id = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(big, "9007199254740993");
}

async fn mysql_like(engine: &str, url: String, extra_tables: &[&str]) {
    let mut conn = match MySqlConnection::connect(&url).await {
        Ok(conn) => conn,
        Err(err) => return unavailable(engine, err),
    };

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = 'tablesail'",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_has_all(engine, &tables, SQL_TABLES);
    assert_has_all(
        engine,
        &tables,
        &["order", "zero_dates", "wide_table", "myisam_table"],
    );
    assert_has_all(engine, &tables, extra_tables);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM big_table")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(rows, BIG_TABLE_ROWS);

    let wide_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns
         WHERE table_schema = 'tablesail' AND table_name = 'wide_table'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(wide_columns, 151);

    let zero_dates: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM zero_dates WHERE CAST(d AS CHAR) = '0000-00-00'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(zero_dates, 1);

    let databases: Vec<String> =
        sqlx::query_scalar("SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_has_all(engine, &databases, &["tablesail", "tablesail_other"]);
}

#[tokio::test]
async fn mysql_nasty_schema() {
    let url = url(
        "TABLESAIL_MYSQL_URL",
        "mysql://tablesail:tablesail@127.0.0.1:23306/tablesail",
    );
    mysql_like("mysql", url, &[]).await;
}

#[tokio::test]
async fn mariadb_nasty_schema() {
    let url = url(
        "TABLESAIL_MARIADB_URL",
        "mysql://tablesail:tablesail@127.0.0.1:23307/tablesail",
    );
    mysql_like(
        "mariadb",
        url,
        &["mariadb_types", "versioned", "invoice_seq"],
    )
    .await;
}

#[tokio::test]
async fn mongo_nasty_schema() {
    let url = url(
        "TABLESAIL_MONGO_URL",
        "mongodb://127.0.0.1:27117/?serverSelectionTimeoutMS=2000&directConnection=true",
    );
    let client = match mongodb::Client::with_uri_str(&url).await {
        Ok(client) => client,
        Err(err) => return unavailable("mongo", err),
    };
    let db = client.database("tablesail");

    let collections = match db.list_collection_names().await {
        Ok(names) => names,
        Err(err) => return unavailable("mongo", err),
    };
    assert_has_all(
        "mongo",
        &collections,
        &[
            "all_types",
            "big_collection",
            "heterogeneous",
            "deep_nesting",
            "select",
            "with space",
            "dots.in.name",
            "customer_totals",
            "capped_log",
            "metrics",
            "empty_collection",
            "seed_done",
        ],
    );

    let docs = db
        .collection::<mongodb::bson::Document>("big_collection")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(docs as i64, BIG_TABLE_ROWS);

    let all_types = db
        .collection::<mongodb::bson::Document>("all_types")
        .find_one(doc! { "string": "hello" })
        .await
        .unwrap()
        .expect("all_types row 1");
    assert!(matches!(
        all_types.get("decimal128"),
        Some(mongodb::bson::Bson::Decimal128(_))
    ));
    assert!(matches!(
        all_types.get("int64"),
        Some(mongodb::bson::Bson::Int64(9007199254740993))
    ));
}
