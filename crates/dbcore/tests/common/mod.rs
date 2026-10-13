//! Shared harness for the live-engine integration tests.
//!
//! Every test runs against one engine through [`run_on`]: it connects a `dbcore`
//! driver (the code under test) and, next to it, an independent "side channel"
//! (plain sqlx for the SQL engines, the raw `mongodb` client for Mongo) that is used
//! to build fixtures and to verify results without trusting the driver. Scratch
//! tables get unique names and are dropped when the test ends, even when it fails,
//! so tests run in parallel and can be re-run. Seeded tables are only read.
//!
//! Servers are the docker-compose databases (see `docker-compose.yml`); URLs can be
//! overridden with `TABLESOFT_{POSTGRES,MYSQL,MARIADB,MONGO}_URL`. An unreachable
//! server skips the test, unless `TABLESOFT_REQUIRE_DBS=1`, which fails it.
//!
//! Each test file uses a different subset of this module.
#![allow(dead_code, unused_macros, unused_imports)]

use std::collections::BTreeMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use dbcore::driver::{Cursor, CursorEvent, Driver};
use dbcore::model::{
    BrowseRequest, ColumnMeta, ConnectParams, ConnectionConfig, EnvTag, Filter, FilterOp, Page,
    Sort, SslConfig, TableRef,
};
use dbcore::{Dialect, Engine, Error, Value};
use futures_util::FutureExt;
use sqlx::{
    AssertSqlSafe, Connection, MySqlConnection, PgConnection, Row as _, SqliteConnection, ValueRef,
};
use tokio::sync::Mutex;

pub type Row = Vec<Value>;

/// Database name every server-engine test works in.
pub const DB: &str = "tablesoft";

pub const SQL: [Engine; 4] = [
    Engine::Sqlite,
    Engine::Postgres,
    Engine::Mysql,
    Engine::Mariadb,
];

// ---------------------------------------------------------------------------
// Test generation
// ---------------------------------------------------------------------------

/// `each_engine!(body_fn, sql)` generates `mod body_fn { sqlite, postgres, mysql, mariadb }`
/// tests that call `async fn body_fn(c: &Ctx)` on the named engines. Groups: `sql`
/// (the four SQL engines), `all` (plus mongo); or list engines: `each_engine!(f, postgres, mysql)`.
macro_rules! engine_test {
    (sqlite, $body:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn sqlite() {
            $crate::common::run_on(::dbcore::Engine::Sqlite, super::$body).await
        }
    };
    (postgres, $body:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn postgres() {
            $crate::common::run_on(::dbcore::Engine::Postgres, super::$body).await
        }
    };
    (mysql, $body:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn mysql() {
            $crate::common::run_on(::dbcore::Engine::Mysql, super::$body).await
        }
    };
    (mariadb, $body:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn mariadb() {
            $crate::common::run_on(::dbcore::Engine::Mariadb, super::$body).await
        }
    };
    (mongo, $body:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn mongo() {
            $crate::common::run_on(::dbcore::Engine::Mongodb, super::$body).await
        }
    };
}

macro_rules! each_engine {
    ($body:ident, sql) => {
        mod $body {
            #[allow(unused_imports)]
            use super::*;
            engine_test!(sqlite, $body);
            engine_test!(postgres, $body);
            engine_test!(mysql, $body);
            engine_test!(mariadb, $body);
        }
    };
    ($body:ident, all) => {
        mod $body {
            #[allow(unused_imports)]
            use super::*;
            engine_test!(sqlite, $body);
            engine_test!(postgres, $body);
            engine_test!(mysql, $body);
            engine_test!(mariadb, $body);
            engine_test!(mongo, $body);
        }
    };
    ($body:ident, $($engine:ident),+) => {
        mod $body {
            #[allow(unused_imports)]
            use super::*;
            $( engine_test!($engine, $body); )+
        }
    };
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

fn env_url(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_owned())
}

pub fn postgres_url() -> String {
    env_url(
        "TABLESOFT_POSTGRES_URL",
        "postgres://tablesoft:tablesoft@127.0.0.1:25432/tablesoft",
    )
}

pub fn mysql_url(engine: Engine) -> String {
    if engine == Engine::Mariadb {
        env_url(
            "TABLESOFT_MARIADB_URL",
            "mysql://tablesoft:tablesoft@127.0.0.1:23307/tablesoft",
        )
    } else {
        env_url(
            "TABLESOFT_MYSQL_URL",
            "mysql://tablesoft:tablesoft@127.0.0.1:23306/tablesoft",
        )
    }
}

pub fn mongo_url() -> String {
    env_url(
        "TABLESOFT_MONGO_URL",
        "mongodb://127.0.0.1:27117/?serverSelectionTimeoutMS=2000&directConnection=true",
    )
}

pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Sqlite => "sqlite",
        Engine::Postgres => "postgres",
        Engine::Mysql => "mysql",
        Engine::Mariadb => "mariadb",
        Engine::Mongodb => "mongo",
    }
}

fn blank(engine: Engine, name: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: String::new(),
        name: name.to_owned(),
        engine,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: String::new(),
        file_path: None,
        url: None,
        ssl: SslConfig::default(),
        ssh: None,
        env: EnvTag::Local,
        color: None,
        read_only: false,
        options: BTreeMap::new(),
    }
}

/// The connection settings for the docker test server of `engine` (for SQLite, `file`
/// is the database file) and the password to go with them.
pub fn config_for(engine: Engine, file: Option<&str>) -> (ConnectionConfig, Option<String>) {
    let mut cfg = blank(engine, "integration test");
    let defaults = match engine {
        Engine::Sqlite => {
            cfg.file_path = Some(file.expect("sqlite needs a file").to_owned());
            return (cfg, None);
        }
        Engine::Postgres => ("127.0.0.1", 25432, std::env::var("TABLESOFT_POSTGRES_URL")),
        Engine::Mysql => ("127.0.0.1", 23306, std::env::var("TABLESOFT_MYSQL_URL")),
        Engine::Mariadb => ("127.0.0.1", 23307, std::env::var("TABLESOFT_MARIADB_URL")),
        Engine::Mongodb => ("127.0.0.1", 27117, std::env::var("TABLESOFT_MONGO_URL")),
    };
    match defaults.2 {
        Ok(url) => {
            cfg.url = Some(url);
            if engine == Engine::Mongodb {
                cfg.database = DB.to_owned();
            }
            (cfg, None)
        }
        Err(_) => {
            cfg.host = defaults.0.to_owned();
            cfg.port = defaults.1;
            cfg.database = DB.to_owned();
            if engine == Engine::Mongodb {
                // The docker Mongo has no authentication.
                (cfg, None)
            } else {
                cfg.user = "tablesoft".to_owned();
                (cfg, Some("tablesoft".to_owned()))
            }
        }
    }
}

pub fn params_of(cfg: ConnectionConfig, password: Option<String>) -> ConnectParams {
    ConnectParams {
        host: cfg.host.clone(),
        port: cfg.port,
        password,
        config: cfg,
    }
}

/// Connects a driver; `tweak` can adjust the config first (options, database…).
pub async fn connect_driver(
    engine: Engine,
    file: Option<&str>,
    tweak: impl FnOnce(&mut ConnectionConfig),
) -> dbcore::Result<Arc<dyn Driver>> {
    let (mut cfg, password) = config_for(engine, file);
    tweak(&mut cfg);
    dbcore::drivers::connect(&params_of(cfg, password)).await
}

/// Handles an unreachable server: skip locally, fail in CI.
fn unavailable(engine: Engine, err: impl std::fmt::Display) {
    if std::env::var("TABLESOFT_REQUIRE_DBS").is_ok_and(|v| v == "1") {
        panic!("{} is required but unreachable: {err}", engine_name(engine));
    }
    eprintln!(
        "SKIPPED {}: {err} (run `docker compose up -d --wait`)",
        engine_name(engine)
    );
}

// ---------------------------------------------------------------------------
// The side channel: independent access to the same database
// ---------------------------------------------------------------------------

enum Side {
    Sqlite(Mutex<SqliteConnection>),
    Postgres(Mutex<PgConnection>),
    Mysql(Mutex<MySqlConnection>),
    Mongo(mongodb::Client),
}

enum Cleanup {
    Sql(String),
    Collection(String),
    Dir(PathBuf),
}

/// One test's connection to one engine, plus bookkeeping for scratch objects.
pub struct Ctx {
    pub engine: Engine,
    pub driver: Arc<dyn Driver>,
    pub dialect: Dialect,
    /// Database name used in `TableRef`s ("main" for SQLite).
    pub database: String,
    /// Schema used in `TableRef`s (`public` for Postgres, none elsewhere).
    pub schema: Option<String>,
    /// Unique prefix of this test's scratch objects.
    pub prefix: String,
    /// SQLite database file.
    pub file: Option<String>,
    side: Side,
    cleanups: StdMutex<Vec<Cleanup>>,
}

/// Runs `body` against `engine`, then drops everything it created, even when it panics.
pub async fn run_on(engine: Engine, body: impl AsyncFnOnce(&Ctx)) {
    let ctx = match Ctx::open(engine).await {
        Ok(ctx) => ctx,
        Err(err) => return unavailable(engine, err),
    };
    let outcome = AssertUnwindSafe(body(&ctx)).catch_unwind().await;
    ctx.teardown().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

impl Ctx {
    async fn open(engine: Engine) -> Result<Ctx, String> {
        let prefix = format!("tst_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let mut cleanups = Vec::new();
        let mut file = None;
        let (side, database, schema) = match engine {
            Engine::Sqlite => {
                let dir = std::env::temp_dir().join(format!("tablesoft-it-{prefix}"));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join("test.sqlite");
                cleanups.push(Cleanup::Dir(dir));
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true);
                let conn = SqliteConnection::connect_with(&options)
                    .await
                    .map_err(|e| e.to_string())?;
                file = Some(path.to_string_lossy().into_owned());
                (Side::Sqlite(Mutex::new(conn)), "main".to_owned(), None)
            }
            Engine::Postgres => {
                let conn = PgConnection::connect(&postgres_url())
                    .await
                    .map_err(|e| e.to_string())?;
                (
                    Side::Postgres(Mutex::new(conn)),
                    DB.to_owned(),
                    Some("public".to_owned()),
                )
            }
            Engine::Mysql | Engine::Mariadb => {
                let conn = MySqlConnection::connect(&mysql_url(engine))
                    .await
                    .map_err(|e| e.to_string())?;
                (Side::Mysql(Mutex::new(conn)), DB.to_owned(), None)
            }
            Engine::Mongodb => {
                let client = mongodb::Client::with_uri_str(&mongo_url())
                    .await
                    .map_err(|e| e.to_string())?;
                // Fail fast when the server is down.
                client
                    .database("admin")
                    .run_command(mongodb::bson::doc! { "ping": 1 })
                    .await
                    .map_err(|e| e.to_string())?;
                (Side::Mongo(client), DB.to_owned(), None)
            }
        };
        let driver = connect_driver(engine, file.as_deref(), |_| {})
            .await
            .map_err(|e| e.to_string())?;
        let dialect = driver.dialect();
        Ok(Ctx {
            engine,
            driver,
            dialect,
            database,
            schema,
            prefix,
            file,
            side,
            cleanups: StdMutex::new(cleanups),
        })
    }

    async fn teardown(&self) {
        let cleanups: Vec<Cleanup> = std::mem::take(&mut *self.cleanups.lock().unwrap());
        // Newest first; a statement that fails (a type still used by a table, say) is
        // retried after the others, so registration order does not matter.
        let mut retry: Vec<String> = Vec::new();
        for cleanup in cleanups.into_iter().rev() {
            match cleanup {
                Cleanup::Sql(sql) => {
                    if self.try_side_exec(&sql).await.is_err() {
                        retry.push(sql);
                    }
                }
                Cleanup::Collection(name) => {
                    if let Side::Mongo(client) = &self.side {
                        let _ = client
                            .database(&self.database)
                            .collection::<mongodb::bson::Document>(&name)
                            .drop()
                            .await;
                    }
                }
                Cleanup::Dir(dir) => {
                    // The driver holds the file open until it is closed.
                    self.driver.close().await;
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
        }
        for _ in 0..3 {
            if retry.is_empty() {
                break;
            }
            let mut still = Vec::new();
            for sql in retry {
                if self.try_side_exec(&sql).await.is_err() {
                    still.push(sql);
                }
            }
            retry = still;
        }
        for sql in retry {
            eprintln!("could not clean up: {sql}");
        }
        self.driver.close().await;
    }

    // --- engine predicates ------------------------------------------------

    pub fn is_sqlite(&self) -> bool {
        self.engine == Engine::Sqlite
    }
    pub fn is_pg(&self) -> bool {
        self.engine == Engine::Postgres
    }
    pub fn is_mysql(&self) -> bool {
        self.engine == Engine::Mysql
    }
    pub fn is_mariadb(&self) -> bool {
        self.engine == Engine::Mariadb
    }
    /// MySQL or MariaDB.
    pub fn is_my(&self) -> bool {
        matches!(self.engine, Engine::Mysql | Engine::Mariadb)
    }
    pub fn is_mongo(&self) -> bool {
        self.engine == Engine::Mongodb
    }

    // --- names ------------------------------------------------------------

    /// A unique scratch name for this test (`tst_<id>_<suffix>`).
    pub fn name(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.prefix)
    }

    pub fn tref(&self, name: &str) -> TableRef {
        TableRef {
            database: self.database.clone(),
            schema: self.schema.clone(),
            name: name.to_owned(),
        }
    }

    /// Quoted identifier.
    pub fn q(&self, ident: &str) -> String {
        self.dialect.quote_ident(ident)
    }

    /// Quoted, qualified table name.
    pub fn qt(&self, name: &str) -> String {
        self.dialect.qualified(&self.tref(name))
    }

    /// String literal.
    pub fn lit(&self, s: &str) -> String {
        self.dialect.quote_string(s)
    }

    /// A browse request with no filters, sort or projection.
    pub fn browse(&self, name: &str) -> BrowseRequest {
        BrowseRequest {
            table: self.tref(name),
            filters: vec![],
            match_all: true,
            raw_where: None,
            sort: vec![],
            columns: vec![],
        }
    }

    // --- scratch bookkeeping ------------------------------------------------

    /// Registers SQL to run (ignoring errors) when the test ends; newest first.
    pub fn cleanup_sql(&self, sql: impl Into<String>) {
        self.cleanups.lock().unwrap().push(Cleanup::Sql(sql.into()));
    }

    /// Registers a Mongo collection to drop when the test ends.
    pub fn cleanup_collection(&self, name: &str) {
        self.cleanups
            .lock()
            .unwrap()
            .push(Cleanup::Collection(name.to_owned()));
    }

    /// A unique table name that is dropped at the end of the test.
    pub fn scratch(&self, suffix: &str) -> String {
        let name = self.name(suffix);
        if self.is_mongo() {
            self.cleanup_collection(&name);
        } else {
            self.cleanup_sql(format!("DROP TABLE IF EXISTS {}", self.qt(&name)));
        }
        name
    }

    /// Like `scratch`, but for a view.
    pub fn scratch_view(&self, suffix: &str) -> String {
        let name = self.name(suffix);
        self.cleanup_sql(format!("DROP VIEW IF EXISTS {}", self.qt(&name)));
        name
    }

    // --- side channel ---------------------------------------------------------

    /// Runs SQL on the side channel; returns the affected-row count. Panics on error.
    pub async fn side_exec(&self, sql: &str) -> u64 {
        match self.try_side_exec(sql).await {
            Ok(n) => n,
            Err(e) => panic!("side channel failed: {e}\n  SQL: {}", clip(sql)),
        }
    }

    pub async fn try_side_exec(&self, sql: &str) -> Result<u64, String> {
        let sql = AssertSqlSafe(sql.to_owned());
        match &self.side {
            Side::Sqlite(c) => sqlx::raw_sql(sql)
                .execute(&mut *c.lock().await)
                .await
                .map(|r| r.rows_affected()),
            Side::Postgres(c) => sqlx::raw_sql(sql)
                .execute(&mut *c.lock().await)
                .await
                .map(|r| r.rows_affected()),
            Side::Mysql(c) => sqlx::raw_sql(sql)
                .execute(&mut *c.lock().await)
                .await
                .map(|r| r.rows_affected()),
            Side::Mongo(_) => panic!("no SQL on Mongo"),
        }
        .map_err(|e| e.to_string())
    }

    /// First column of the first row as text (`None` for NULL or no rows).
    pub async fn side_text(&self, sql: &str) -> Option<String> {
        self.side_column(sql).await.into_iter().next().flatten()
    }

    /// First column of every row, as text.
    pub async fn side_column(&self, sql: &str) -> Vec<Option<String>> {
        let sql = AssertSqlSafe(sql.to_owned());
        let shown = clip(sql.0.as_str());
        match &self.side {
            Side::Sqlite(c) => {
                let rows = sqlx::raw_sql(sql)
                    .fetch_all(&mut *c.lock().await)
                    .await
                    .unwrap_or_else(|e| panic!("side query failed: {e}\n  SQL: {shown}"));
                rows.iter()
                    .map(|r| {
                        if r.try_get_raw(0).unwrap().is_null() {
                            None
                        } else {
                            Some(r.try_get_unchecked::<String, _>(0).unwrap())
                        }
                    })
                    .collect()
            }
            Side::Postgres(c) => {
                let rows = sqlx::raw_sql(sql)
                    .fetch_all(&mut *c.lock().await)
                    .await
                    .unwrap_or_else(|e| panic!("side query failed: {e}\n  SQL: {shown}"));
                rows.iter()
                    .map(|r| {
                        let raw = r.try_get_raw(0).unwrap();
                        if raw.is_null() {
                            None
                        } else {
                            Some(raw.as_str().unwrap().to_owned())
                        }
                    })
                    .collect()
            }
            Side::Mysql(c) => {
                let rows = sqlx::raw_sql(sql)
                    .fetch_all(&mut *c.lock().await)
                    .await
                    .unwrap_or_else(|e| panic!("side query failed: {e}\n  SQL: {shown}"));
                rows.iter()
                    .map(|r| {
                        if r.try_get_raw(0).unwrap().is_null() {
                            None
                        } else {
                            let b: &[u8] = r.try_get_unchecked(0).unwrap();
                            Some(String::from_utf8_lossy(b).into_owned())
                        }
                    })
                    .collect()
            }
            Side::Mongo(_) => panic!("no SQL on Mongo"),
        }
    }

    /// Row count of a table through the side channel (SQL) or the raw client (Mongo).
    pub async fn side_count(&self, table: &str) -> i64 {
        if let Side::Mongo(client) = &self.side {
            return client
                .database(&self.database)
                .collection::<mongodb::bson::Document>(table)
                .count_documents(mongodb::bson::doc! {})
                .await
                .unwrap() as i64;
        }
        self.side_text(&format!("SELECT COUNT(*) FROM {}", self.qt(table)))
            .await
            .expect("count")
            .parse()
            .expect("count is a number")
    }

    /// The raw Mongo database handle.
    pub fn mongo_db(&self) -> mongodb::Database {
        match &self.side {
            Side::Mongo(client) => client.database(&self.database),
            _ => panic!("not a Mongo test"),
        }
    }

    pub fn mongo_client(&self) -> mongodb::Client {
        match &self.side {
            Side::Mongo(client) => client.clone(),
            _ => panic!("not a Mongo test"),
        }
    }

    pub fn mongo_coll(&self, name: &str) -> mongodb::Collection<mongodb::bson::Document> {
        self.mongo_db().collection(name)
    }

    // --- running statements through the driver -------------------------------

    /// Runs one statement through `Driver::query` and drains it.
    pub async fn events(&self, sql: &str) -> dbcore::Result<Vec<CursorEvent>> {
        drain(self.driver.query(Some(&self.database), sql).await?).await
    }

    /// Like `events`, but panics on error.
    pub async fn run(&self, sql: &str) -> Vec<CursorEvent> {
        match self.events(sql).await {
            Ok(e) => e,
            Err(e) => panic!("query failed: {e:?}\n  SQL: {}", clip(sql)),
        }
    }

    /// Rows of the first result set of `sql`.
    pub async fn rows(&self, sql: &str) -> Vec<Row> {
        let sets = sets_of(&self.run(sql).await);
        sets.into_iter().next().map(|s| s.rows).unwrap_or_default()
    }

    /// The single cell of a one-row, one-column result.
    pub async fn scalar(&self, sql: &str) -> Value {
        let rows = self.rows(sql).await;
        assert_eq!(rows.len(), 1, "expected one row from {sql}: {rows:?}");
        assert_eq!(rows[0].len(), 1, "expected one column from {sql}");
        rows[0][0].clone()
    }

    /// Affected rows of a DML/DDL statement run through the driver.
    pub async fn affected(&self, sql: &str) -> u64 {
        let events = self.run(sql).await;
        events
            .iter()
            .find_map(|e| match e {
                CursorEvent::Affected(n) => Some(*n),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Affected event from {sql}: {events:?}"))
    }

    // --- tables used by many tests -------------------------------------------

    /// Creates the "items" fixture (id PK, name NOT NULL, grp NOT NULL, label NULL) with
    /// rows `item(1..=n)` through the side channel and returns its name.
    pub async fn make_items(&self, suffix: &str, n: i64) -> String {
        let name = self.scratch(suffix);
        let t = self.qt(&name);
        let ddl = match self.engine {
            Engine::Sqlite => format!(
                "CREATE TABLE {t} (id INTEGER PRIMARY KEY, name TEXT NOT NULL, grp INTEGER NOT NULL, label TEXT)"
            ),
            Engine::Postgres => format!(
                "CREATE TABLE {t} (id integer PRIMARY KEY, name text NOT NULL, grp integer NOT NULL, label text)"
            ),
            _ => format!(
                "CREATE TABLE {t} (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL, grp INT NOT NULL, label VARCHAR(100) NULL)"
            ),
        };
        self.side_exec(&ddl).await;
        self.insert_items(&name, 1..=n).await;
        name
    }

    /// Inserts `item(id)` for each id through the side channel.
    pub async fn insert_items(&self, name: &str, ids: impl IntoIterator<Item = i64>) {
        let t = self.qt(name);
        let ids: Vec<i64> = ids.into_iter().collect();
        for chunk in ids.chunks(500) {
            let values: Vec<String> = chunk
                .iter()
                .map(|&id| {
                    let it = item(id);
                    format!(
                        "({}, '{}', {}, {})",
                        it.id,
                        it.name,
                        it.grp,
                        it.label
                            .as_ref()
                            .map_or("NULL".to_owned(), |l| format!("'{l}'"))
                    )
                })
                .collect();
            self.side_exec(&format!(
                "INSERT INTO {t} (id, name, grp, label) VALUES {}",
                values.join(", ")
            ))
            .await;
        }
    }

    /// Browse page through the driver, panicking on error.
    pub async fn page(&self, req: &BrowseRequest, offset: u64, limit: u32) -> Page {
        match self.driver.browse_page(req, offset, limit).await {
            Ok(p) => p,
            Err(e) => panic!("browse_page failed: {e:?}\n  request: {req:?}"),
        }
    }

    /// Reads a page up to three times and returns the page with the fastest time: a latency
    /// check should measure the driver, not a one-off stall of a busy machine.
    pub async fn fastest_page(
        &self,
        req: &BrowseRequest,
        offset: u64,
        limit: u32,
    ) -> (Page, Duration) {
        let mut best: Option<(Page, Duration)> = None;
        for _ in 0..3 {
            let started = Instant::now();
            let page = self.page(req, offset, limit).await;
            let took = started.elapsed();
            if best.as_ref().is_none_or(|(_, t)| took < *t) {
                best = Some((page, took));
            }
            if took < Duration::from_millis(300) {
                break;
            }
        }
        best.expect("at least one attempt")
    }

    /// Ids (first column) of a page.
    pub async fn page_ids(&self, req: &BrowseRequest, offset: u64, limit: u32) -> Vec<i64> {
        ids(&self.page(req, offset, limit).await.rows, 0)
    }

    /// Reads `req` completely through `browse_all`.
    pub async fn all_rows(&self, req: &BrowseRequest) -> Vec<Row> {
        let cursor = self
            .driver
            .browse_all(req)
            .await
            .unwrap_or_else(|e| panic!("browse_all failed: {e:?}"));
        let events = drain(cursor).await.unwrap();
        sets_of(&events)
            .into_iter()
            .next()
            .map(|s| s.rows)
            .unwrap_or_default()
    }
}

fn clip(sql: &str) -> String {
    if sql.chars().count() > 600 {
        format!("{}…", sql.chars().take(600).collect::<String>())
    } else {
        sql.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Fixture data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i64,
    pub name: String,
    pub grp: i64,
    pub label: Option<String>,
}

/// Deterministic content of row `id` of the items fixture.
pub fn item(id: i64) -> Item {
    Item {
        id,
        name: format!("item {id:05}"),
        grp: id % 7,
        label: (id % 5 != 0).then(|| format!("lbl-{}", id % 13)),
    }
}

/// An item as the grid shows it.
pub fn item_row(id: i64) -> Row {
    let it = item(id);
    vec![
        Value::Int(it.id),
        Value::Text(it.name),
        Value::Int(it.grp),
        it.label.map_or(Value::Null, Value::Text),
    ]
}

// ---------------------------------------------------------------------------
// Cursor helpers
// ---------------------------------------------------------------------------

/// Reads a cursor to the end (batches of up to 5000 rows).
pub async fn drain(mut cursor: Box<dyn Cursor>) -> dbcore::Result<Vec<CursorEvent>> {
    let mut out = Vec::new();
    while let Some(event) = cursor.next(5000).await? {
        out.push(event);
    }
    Ok(out)
}

/// One result set of a statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Set {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
}

impl Set {
    pub fn names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// The result sets in `events` (a `Columns` event starts one; `Rows` fill it).
pub fn sets_of(events: &[CursorEvent]) -> Vec<Set> {
    let mut sets: Vec<Set> = Vec::new();
    for event in events {
        match event {
            CursorEvent::Columns(columns) => sets.push(Set {
                columns: columns.clone(),
                rows: Vec::new(),
            }),
            CursorEvent::Rows(rows) => sets
                .last_mut()
                .expect("Rows before Columns")
                .rows
                .extend(rows.iter().cloned()),
            CursorEvent::Affected(_) => {}
        }
    }
    sets
}

/// The `Affected` counts in `events`, in order.
pub fn affected_of(events: &[CursorEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            CursorEvent::Affected(n) => Some(*n),
            _ => None,
        })
        .collect()
}

/// Integers of column `col`.
pub fn ids(rows: &[Row], col: usize) -> Vec<i64> {
    rows.iter()
        .map(|r| match &r[col] {
            Value::Int(i) => *i,
            Value::UInt(u) => *u as i64,
            other => panic!("expected an integer in column {col}, got {other:?}"),
        })
        .collect()
}

/// Text of column `col`.
pub fn texts(rows: &[Row], col: usize) -> Vec<String> {
    rows.iter()
        .map(|r| match &r[col] {
            Value::Text(s) => s.clone(),
            other => panic!("expected text in column {col}, got {other:?}"),
        })
        .collect()
}

/// Splits `script` like the app does and runs the statements one after another until
/// one fails. Returns one entry per statement that was started.
pub async fn run_script(c: &Ctx, script: &str) -> Vec<(String, dbcore::Result<Vec<CursorEvent>>)> {
    let spans = dbcore::split::split_statements(script, c.dialect);
    let mut out = Vec::new();
    for span in spans {
        let result = c.events(&span.sql).await;
        let failed = result.is_err();
        out.push((span.sql, result));
        if failed {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Filters and misc
// ---------------------------------------------------------------------------

pub fn filter(column: &str, op: FilterOp, value: Option<&str>) -> Filter {
    Filter {
        column: column.to_owned(),
        op,
        value: value.map(str::to_owned),
    }
}

pub fn sort(column: &str, descending: bool) -> Sort {
    Sort {
        column: column.to_owned(),
        descending,
    }
}

/// Polls `check` until it returns true or `timeout` passes (then panics with `what`).
pub async fn eventually<F, Fut>(timeout: Duration, what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = Instant::now();
    loop {
        if check().await {
            return;
        }
        if started.elapsed() > timeout {
            panic!("timed out after {timeout:?} waiting for: {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The database error code of `err`, if it is one.
pub fn db_code(err: &Error) -> Option<&str> {
    match err {
        Error::Database { code, .. } => code.as_deref(),
        _ => None,
    }
}

/// The message of a database error.
pub fn db_message(err: &Error) -> &str {
    match err {
        Error::Database { message, .. } => message,
        other => panic!("expected a database error, got {other:?}"),
    }
}
