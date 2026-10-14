//! Catalog and browsing against every SQL engine: connect, list, structure, paging,
//! sorting, filters, counts. See `tests/common/mod.rs` for the harness.

#[macro_use]
mod common;

use std::time::{Duration, Instant};

use common::*;
use dbcore::model::{
    Editability, Filter, FilterOp, Secrets, TableInfo, TableKind, TableRef, TypeFamily,
};
use dbcore::{Dialect, Engine, Error, Session, Value};

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

async fn connect_ping_version(c: &Ctx) {
    let version = c.driver.server_version().await.unwrap();
    let expected = match c.engine {
        Engine::Sqlite => "SQLite 3.",
        Engine::Postgres => "PostgreSQL 1",
        Engine::Mysql => "8.",
        Engine::Mariadb => "1",
        Engine::Mongodb => unreachable!(),
    };
    assert!(version.starts_with(expected), "version {version:?}");
    match c.engine {
        Engine::Mysql => assert!(version.ends_with("-MySQL"), "{version}"),
        Engine::Mariadb => assert!(version.ends_with("-MariaDB"), "{version}"),
        _ => {}
    }
    assert_eq!(c.driver.engine(), c.engine);
    assert_eq!(c.driver.dialect(), Dialect::for_engine(c.engine));

    // "Ping": a trivial round trip on the query path.
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
    let started = Instant::now();
    for _ in 0..20 {
        assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
    }
    assert!(started.elapsed() < Duration::from_secs(5));
}
each_engine!(connect_ping_version, sql);

async fn session_open_reports_version(c: &Ctx) {
    let (cfg, password) = config_for(c.engine, c.file.as_deref());
    let session = Session::open(
        cfg,
        Secrets {
            db_password: password,
            ..Secrets::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        session.server_version,
        c.driver.server_version().await.unwrap()
    );
    assert!(session.tunnel.is_none());
    assert_eq!(session.driver.engine(), c.engine);
    session.close().await;
}
each_engine!(session_open_reports_version, sql);

async fn connect_through_a_url(c: &Ctx) {
    // A full URL overrides host/port/user/database.
    let (mut cfg, password) = config_for(c.engine, None);
    if cfg.url.is_some() {
        return eprintln!("SKIPPED: the server URL is already set by the environment");
    }
    let (user, port) = (cfg.user.clone(), cfg.port);
    let url = if c.is_pg() {
        format!("postgres://{user}:tablesail@127.0.0.1:{port}/tablesail")
    } else {
        format!("mysql://{user}:tablesail@127.0.0.1:{port}/tablesail")
    };
    cfg.url = Some(url);
    cfg.host = String::new();
    cfg.port = 0;
    cfg.user = String::new();
    cfg.database = String::new();
    let driver = dbcore::drivers::connect(&params_of(cfg, password))
        .await
        .unwrap();
    let rows = drain(driver.query(None, "SELECT 1").await.unwrap())
        .await
        .unwrap();
    assert_eq!(sets_of(&rows)[0].rows, vec![vec![Value::Int(1)]]);
    driver.close().await;
}
each_engine!(connect_through_a_url, postgres, mysql, mariadb);

async fn bad_credentials_and_unreachable_servers_fail_fast(c: &Ctx) {
    let started = Instant::now();
    // Sanity: the normal credentials connect.
    connect_driver(c.engine, None, |_| {})
        .await
        .expect("the normal credentials connect")
        .close()
        .await;

    // Wrong password: a database error from the server, not a hang.
    let (mut cfg, _) = config_for(c.engine, None);
    if cfg.url.is_some() {
        return eprintln!("SKIPPED: the server URL is already set by the environment");
    }
    let err = dbcore::drivers::connect(&params_of(cfg.clone(), Some("not-the-password".into())))
        .await
        .err()
        .expect("a wrong password must fail");
    assert!(
        matches!(err, Error::Database { .. } | Error::Connection(_)),
        "{err:?}"
    );

    // Nothing listens on port 1.
    cfg.port = 1;
    let err = dbcore::drivers::connect(&params_of(cfg, Some("tablesail".into())))
        .await
        .err()
        .expect("a closed port must fail");
    assert!(matches!(err, Error::Connection(_)), "{err:?}");

    // A database that does not exist.
    let err = connect_driver(c.engine, None, |cfg| {
        cfg.database = "no_such_database_xyz".into()
    })
    .await
    .err()
    .expect("an unknown database must fail");
    assert!(
        matches!(err, Error::Database { .. } | Error::Connection(_)),
        "{err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "failures took {:?}",
        started.elapsed()
    );
}
each_engine!(
    bad_credentials_and_unreachable_servers_fail_fast,
    postgres,
    mysql,
    mariadb
);

async fn pg_connection_options_apply(c: &Ctx) {
    // search_path, statement_timeout and the application name reach the session.
    let app = c.name("app");
    let driver = connect_driver(Engine::Postgres, None, |cfg| {
        cfg.options
            .insert("search_path".into(), "billing, public".into());
        cfg.options.insert("statement_timeout".into(), "700".into());
        cfg.options.insert("application_name".into(), app.clone());
        cfg.options.insert("timezone".into(), "Asia/Tokyo".into());
    })
    .await
    .unwrap();
    let show = |name: &'static str| {
        let driver = driver.clone();
        async move {
            let events = drain(driver.query(None, &format!("SHOW {name}")).await.unwrap())
                .await
                .unwrap();
            sets_of(&events)[0].rows[0][0].clone()
        }
    };
    assert_eq!(
        show("search_path").await,
        Value::Text("billing, public".into())
    );
    assert_eq!(show("application_name").await, Value::Text(app.clone()));
    assert_eq!(show("timezone").await, Value::Text("Asia/Tokyo".into()));
    assert_eq!(show("statement_timeout").await, Value::Text("700ms".into()));
    // Unqualified names resolve through the search path.
    let billing = c.driver.list_tables(DB, Some("billing")).await.unwrap();
    assert!(billing.iter().any(|t| t.table.name == "invoices"));
    let events = drain(
        driver
            .query(None, "SELECT count(*) >= 0 FROM invoices")
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(sets_of(&events)[0].rows[0][0], Value::Bool(true));
    // The statement timeout is enforced by the server.
    let started = Instant::now();
    let err = drain(driver.query(None, "SELECT pg_sleep(10)").await.unwrap())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "57014"),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // The connection works afterwards.
    let events = drain(driver.query(None, "SELECT 1").await.unwrap())
        .await
        .unwrap();
    assert_eq!(sets_of(&events)[0].rows[0][0], Value::Int(1));
    driver.close().await;
}
each_engine!(pg_connection_options_apply, postgres);

#[tokio::test]
async fn sqlite_connect_failures() {
    let dir = std::env::temp_dir().join(format!("tablesail-it-connect-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    // Missing file: reported, not silently created.
    let missing = dir.join("missing.sqlite");
    let err = connect_driver(Engine::Sqlite, missing.to_str(), |_| {})
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::Connection(_)), "{err:?}");
    assert!(!missing.exists());

    // No path at all.
    let err = connect_driver(Engine::Sqlite, Some("  "), |_| {})
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");

    // A file that is not a database.
    let junk = dir.join("junk.sqlite");
    std::fs::write(
        &junk,
        "this is definitely not a sqlite database, just text".repeat(50),
    )
    .unwrap();
    let err = connect_driver(Engine::Sqlite, junk.to_str(), |_| {})
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            Error::Database { .. } | Error::Other(_) | Error::Connection(_)
        ),
        "{err:?}"
    );

    // An in-memory database works and is private to the driver.
    let mem = connect_driver(Engine::Sqlite, Some(":memory:"), |_| {})
        .await
        .unwrap();
    drain(mem.query(None, "CREATE TABLE t (a)").await.unwrap())
        .await
        .unwrap();
    let tables = mem.list_tables("main", None).await.unwrap();
    assert_eq!(tables.len(), 1);
    mem.close().await;
    std::fs::remove_dir_all(dir).ok();
}

// ---------------------------------------------------------------------------
// Catalog: databases, schemas, tables, views
// ---------------------------------------------------------------------------

async fn lists_databases_and_schemas(c: &Ctx) {
    let databases = c.driver.list_databases().await.unwrap();
    let schemas = c.driver.list_schemas(&c.database).await.unwrap();
    match c.engine {
        Engine::Sqlite => {
            assert_eq!(databases, ["main"]);
            assert!(schemas.is_empty());
        }
        Engine::Postgres => {
            assert!(databases.contains(&DB.to_owned()), "{databases:?}");
            assert!(
                !databases.iter().any(|d| d.starts_with("template")),
                "templates are hidden: {databases:?}"
            );
            for s in ["public", "billing", "Mixed Case Schema", "empty_schema"] {
                assert!(schemas.contains(&s.to_owned()), "{schemas:?}");
            }
            for s in ["pg_catalog", "information_schema", "pg_toast"] {
                assert!(!schemas.contains(&s.to_owned()), "{schemas:?}");
            }
            let mut sorted = schemas.clone();
            sorted.sort();
            assert_eq!(schemas, sorted, "schemas come back sorted");
        }
        _ => {
            for d in [DB, "tablesail_other"] {
                assert!(databases.contains(&d.to_owned()), "{databases:?}");
            }
            for d in ["information_schema", "performance_schema", "mysql", "sys"] {
                assert!(
                    !databases.contains(&d.to_owned()),
                    "system database {d} is hidden: {databases:?}"
                );
            }
            assert!(schemas.is_empty());
        }
    }
}
each_engine!(lists_databases_and_schemas, sql);

fn find<'a>(tables: &'a [TableInfo], name: &str) -> &'a TableInfo {
    tables
        .iter()
        .find(|t| t.table.name == name)
        .unwrap_or_else(|| {
            panic!(
                "`{name}` not listed in {:?}",
                tables.iter().map(|t| &t.table.name).collect::<Vec<_>>()
            )
        })
}

async fn lists_tables_and_views(c: &Ctx) {
    let table = c.make_items("a_table", 12).await;
    let view = c.scratch_view("b_view");
    c.side_exec(&format!(
        "CREATE VIEW {} AS SELECT id, name FROM {}",
        c.qt(&view),
        c.qt(&table)
    ))
    .await;
    let comment = match c.engine {
        Engine::Postgres => {
            c.side_exec(&format!("COMMENT ON TABLE {} IS 'the items'", c.qt(&table)))
                .await;
            Some("the items")
        }
        Engine::Mysql | Engine::Mariadb => {
            c.side_exec(&format!(
                "ALTER TABLE {} COMMENT = 'the items'",
                c.qt(&table)
            ))
            .await;
            Some("the items")
        }
        _ => None,
    };
    let mut matview = None;
    if c.is_pg() {
        let name = c.name("c_matview");
        c.cleanup_sql(format!("DROP MATERIALIZED VIEW IF EXISTS {}", c.qt(&name)));
        c.side_exec(&format!(
            "CREATE MATERIALIZED VIEW {} AS SELECT id FROM {}",
            c.qt(&name),
            c.qt(&table)
        ))
        .await;
        matview = Some(name);
    }

    let tables = c
        .driver
        .list_tables(&c.database, c.schema.as_deref())
        .await
        .unwrap();
    let t = find(&tables, &table);
    assert_eq!(t.kind, TableKind::Table);
    assert_eq!(t.table.database, c.database);
    assert_eq!(t.table.schema, c.schema);
    assert_eq!(t.comment.as_deref(), comment);
    let v = find(&tables, &view);
    assert_eq!(v.kind, TableKind::View);
    assert_eq!(v.estimated_rows, None, "views have no row estimate");
    assert_eq!(v.comment, None);
    if let Some(m) = &matview {
        assert_eq!(find(&tables, m).kind, TableKind::MaterializedView);
    }

    // Sorted by name.
    let names: Vec<&str> = tables.iter().map(|t| t.table.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    let mine: Vec<&&str> = names.iter().filter(|n| n.starts_with(&c.prefix)).collect();
    let mut mine_sorted = mine.clone();
    mine_sorted.sort();
    assert_eq!(mine, mine_sorted, "listing is sorted by name");

    // Nothing is listed twice.
    let mut dedup = names.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(dedup.len(), names.len());
}
each_engine!(lists_tables_and_views, sql);

async fn sqlite_hides_shadow_tables_and_shows_virtual_tables(c: &Ctx) {
    let fts = c.name("docs");
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.q(&fts)));
    c.side_exec(&format!(
        "CREATE VIRTUAL TABLE {} USING fts5(body)",
        c.q(&fts)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} (body) VALUES ('hello world')",
        c.q(&fts)
    ))
    .await;
    let plain = c.make_items("plain", 3).await;
    let tables = c.driver.list_tables("main", None).await.unwrap();
    assert_eq!(find(&tables, &fts).kind, TableKind::Virtual);
    assert_eq!(find(&tables, &plain).kind, TableKind::Table);
    for suffix in ["data", "idx", "content", "docsize", "config"] {
        let shadow = format!("{fts}_{suffix}");
        assert!(
            !tables.iter().any(|t| t.table.name == shadow),
            "shadow table {shadow} must be hidden"
        );
    }
    assert!(!tables.iter().any(|t| t.table.name.starts_with("sqlite_")));

    let s = c.driver.table_structure(&c.tref(&fts)).await.unwrap();
    assert_eq!(s.kind, TableKind::Virtual);
    assert_eq!(
        s.editability,
        Editability::ReadOnly("Virtual tables can't be edited.".into())
    );
    let page = c.page(&c.browse(&fts), 0, 10).await;
    assert_eq!(page.rows.len(), 1);
}
each_engine!(sqlite_hides_shadow_tables_and_shows_virtual_tables, sqlite);

async fn seeded_catalog(c: &Ctx) {
    // Read-only checks against the docker "nasty schema".
    let tables = c
        .driver
        .list_tables(&c.database, c.schema.as_deref())
        .await
        .unwrap();
    for name in [
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
    ] {
        find(&tables, name);
    }
    assert_eq!(find(&tables, "customer_totals").kind, TableKind::View);
    assert_eq!(find(&tables, "big_table").kind, TableKind::Table);
    let est = find(&tables, "big_table").estimated_rows.expect("estimate");
    // InnoDB's statistics are rough (about 2x off here); Postgres is close.
    assert!(
        (500_000..5_000_000).contains(&est),
        "big_table estimate {est}"
    );
    if c.is_pg() {
        assert_eq!(
            find(&tables, "mood_counts").kind,
            TableKind::MaterializedView
        );
        // Partitioned parents and their partitions are tables.
        assert_eq!(find(&tables, "events").kind, TableKind::Table);
        assert_eq!(find(&tables, "events_2024").kind, TableKind::Table);
        // Other schemas.
        let billing = c.driver.list_tables(DB, Some("billing")).await.unwrap();
        find(&billing, "invoices");
        let mixed = c
            .driver
            .list_tables(DB, Some("Mixed Case Schema"))
            .await
            .unwrap();
        find(&mixed, "select");
        assert!(
            c.driver
                .list_tables(DB, Some("empty_schema"))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            c.driver
                .list_tables(DB, Some("no_such_schema"))
                .await
                .unwrap()
                .is_empty()
        );
    } else {
        // A second database on the same server.
        let other = c.driver.list_tables("tablesail_other", None).await.unwrap();
        find(&other, "notes");
        assert_eq!(find(&other, "notes").table.database, "tablesail_other");
        let s = c
            .driver
            .table_structure(&TableRef {
                database: "tablesail_other".into(),
                schema: None,
                name: "notes".into(),
            })
            .await
            .unwrap();
        assert_eq!(s.primary_key, ["id"]);
        assert!(
            c.driver
                .list_tables("no_such_db_xyz", None)
                .await
                .unwrap()
                .is_empty()
        );
        if c.is_mariadb() {
            assert_eq!(find(&tables, "invoice_seq").kind, TableKind::Sequence);
        }
    }
}
each_engine!(seeded_catalog, postgres, mysql, mariadb);

// ---------------------------------------------------------------------------
// table_structure
// ---------------------------------------------------------------------------

/// A table with a bit of everything; returns (table, enum type name for Postgres).
async fn rich_table(c: &Ctx) -> String {
    let name = c.scratch("rich");
    let t = c.qt(&name);
    let unique = c.q(&c.name("rich_title_uq"));
    let secondary = c.q(&c.name("rich_price_ix"));
    match c.engine {
        Engine::Postgres => {
            let ty = c.name("rich_state");
            c.cleanup_sql(format!("DROP TYPE IF EXISTS {}", c.q(&ty)));
            c.side_exec(&format!(
                "CREATE TYPE {} AS ENUM ('draft', 'published', 'archived with space', 'it''s')",
                c.q(&ty)
            ))
            .await;
            c.side_exec(&format!(
                "CREATE TABLE {t} (
                    id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                    title varchar(80) NOT NULL DEFAULT 'untitled',
                    price numeric(12,2) NOT NULL DEFAULT 0,
                    state {} NOT NULL DEFAULT 'draft',
                    tags text[],
                    meta jsonb DEFAULT '{{}}'::jsonb,
                    created timestamptz DEFAULT now(),
                    note text,
                    total numeric GENERATED ALWAYS AS (price * 2) STORED
                )",
                c.q(&ty)
            ))
            .await;
            c.side_exec(&format!("COMMENT ON COLUMN {t}.title IS 'the title'"))
                .await;
            c.side_exec(&format!("CREATE UNIQUE INDEX {unique} ON {t} (title)"))
                .await;
            c.side_exec(&format!("CREATE INDEX {secondary} ON {t} USING gin (meta)"))
                .await;
        }
        Engine::Mysql | Engine::Mariadb => {
            c.side_exec(&format!(
                "CREATE TABLE {t} (
                    id INT NOT NULL AUTO_INCREMENT,
                    title VARCHAR(80) NOT NULL DEFAULT 'untitled' COMMENT 'the title',
                    price DECIMAL(12,2) NOT NULL DEFAULT 0,
                    state ENUM('draft', 'published', 'archived with space', 'it''s') NOT NULL DEFAULT 'draft',
                    tags SET('a', 'b', 'c'),
                    meta JSON,
                    created TIMESTAMP NULL DEFAULT CURRENT_TIMESTAMP,
                    note TEXT,
                    total DECIMAL(12,2) GENERATED ALWAYS AS (price * 2) STORED,
                    PRIMARY KEY (id),
                    UNIQUE KEY {unique} (title),
                    KEY {secondary} (price, created)
                )"
            ))
            .await;
        }
        _ => {
            c.side_exec(&format!(
                "CREATE TABLE {t} (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title VARCHAR(80) NOT NULL DEFAULT 'untitled',
                    price NUMERIC(12,2) NOT NULL DEFAULT 0,
                    state TEXT NOT NULL DEFAULT 'draft',
                    tags TEXT,
                    meta JSON,
                    created DATETIME DEFAULT CURRENT_TIMESTAMP,
                    note TEXT,
                    total NUMERIC GENERATED ALWAYS AS (price * 2) VIRTUAL
                )"
            ))
            .await;
            c.side_exec(&format!("CREATE UNIQUE INDEX {unique} ON {t} (title)"))
                .await;
            c.side_exec(&format!("CREATE INDEX {secondary} ON {t} (price, created)"))
                .await;
        }
    }
    name
}

async fn structure_columns_and_indexes(c: &Ctx) {
    let name = rich_table(c).await;
    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    assert_eq!(s.table.name, name);
    assert_eq!(s.kind, TableKind::Table);
    assert_eq!(s.primary_key, ["id"]);
    assert_eq!(s.editability, Editability::Editable);
    assert!(s.foreign_keys.is_empty());
    let names: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "id", "title", "price", "state", "tags", "meta", "created", "note", "total"
        ],
        "columns come in declaration order"
    );
    let col = |n: &str| s.columns.iter().find(|c| c.name == n).unwrap();

    let id = col("id");
    assert!(
        id.is_primary_key && id.is_auto_increment && !id.nullable,
        "{id:?}"
    );
    assert_eq!(id.family, TypeFamily::Integer);
    for other in [
        "title", "price", "state", "tags", "meta", "created", "note", "total",
    ] {
        assert!(!col(other).is_primary_key, "{other}");
    }

    let title = col("title");
    assert_eq!(title.family, TypeFamily::Text);
    assert!(!title.nullable);
    assert!(
        title
            .default
            .as_deref()
            .is_some_and(|d| d.contains("untitled")),
        "{:?}",
        title.default
    );
    assert!(
        title.data_type.to_lowercase().contains("varchar")
            || title.data_type.contains("character varying")
    );
    assert!(title.data_type.contains("80"), "{}", title.data_type);
    assert!(!title.is_auto_increment && !title.is_generated);

    let price = col("price");
    assert!(!price.nullable);
    // `DEFAULT 0` on DECIMAL(12,2) is reported as "0.00" by MySQL and MariaDB.
    assert_eq!(
        price
            .default
            .as_deref()
            .map(|d| d.trim_matches('\'').parse::<f64>().unwrap()),
        Some(0.0)
    );
    if !c.is_sqlite() {
        assert_eq!(price.family, TypeFamily::Decimal);
        assert!(price.data_type.contains("12,2"), "{}", price.data_type);
    }

    let note = col("note");
    assert!(note.nullable && note.default.is_none(), "{note:?}");
    assert_eq!(note.family, TypeFamily::Text);

    let total = col("total");
    assert!(total.is_generated, "{total:?}");

    let created = col("created");
    assert_eq!(created.family, TypeFamily::DateTime);
    assert!(created.nullable);
    assert!(
        created
            .default
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains("now()")
                || d.to_lowercase().contains("current_timestamp")),
        "{:?}",
        created.default
    );

    let state = col("state");
    if !c.is_sqlite() {
        assert_eq!(state.family, TypeFamily::Enum);
        assert_eq!(
            state.enum_values.as_deref(),
            Some(
                &[
                    "draft".to_owned(),
                    "published".into(),
                    "archived with space".into(),
                    "it's".into()
                ][..]
            ),
        );
        assert!(!state.nullable);
        assert!(
            state
                .default
                .as_deref()
                .is_some_and(|d| d.contains("draft"))
        );
    } else {
        assert_eq!(state.family, TypeFamily::Text);
        assert!(state.enum_values.is_none());
    }
    assert!(col("title").enum_values.is_none());

    match c.engine {
        Engine::Postgres => {
            assert_eq!(col("tags").family, TypeFamily::Array);
            assert_eq!(col("tags").data_type, "text[]");
            assert_eq!(col("meta").family, TypeFamily::Json);
            assert_eq!(col("meta").data_type, "jsonb");
            assert!(
                col("meta")
                    .default
                    .as_deref()
                    .is_some_and(|d| d.contains("{}"))
            );
            assert_eq!(title.comment.as_deref(), Some("the title"));
            assert_eq!(col("note").comment, None);
        }
        Engine::Mysql | Engine::Mariadb => {
            assert_eq!(col("tags").family, TypeFamily::Enum);
            assert_eq!(
                col("tags").enum_values.as_deref(),
                Some(&["a".to_owned(), "b".into(), "c".into()][..])
            );
            if c.is_mysql() {
                assert_eq!(col("meta").family, TypeFamily::Json);
            }
            assert_eq!(title.comment.as_deref(), Some("the title"));
            assert_eq!(col("note").comment, None);
        }
        _ => {
            assert_eq!(col("meta").family, TypeFamily::Json);
            assert_eq!(col("price").data_type, "NUMERIC(12,2)");
            assert_eq!(col("price").family, TypeFamily::Decimal);
        }
    }

    // Indexes: the primary key first, then the others with their columns in key order.
    assert!(s.indexes[0].primary, "{:?}", s.indexes);
    assert_eq!(s.indexes[0].columns, ["id"]);
    assert!(s.indexes[0].unique);
    let unique = s
        .indexes
        .iter()
        .find(|i| i.name == c.name("rich_title_uq"))
        .unwrap_or_else(|| panic!("unique index missing: {:?}", s.indexes));
    assert!(unique.unique && !unique.primary);
    assert_eq!(unique.columns, ["title"]);
    let secondary = s
        .indexes
        .iter()
        .find(|i| i.name == c.name("rich_price_ix"))
        .unwrap_or_else(|| panic!("secondary index missing: {:?}", s.indexes));
    assert!(!secondary.unique && !secondary.primary);
    match c.engine {
        Engine::Postgres => {
            assert_eq!(secondary.columns, ["meta"]);
            assert_eq!(secondary.method.as_deref(), Some("gin"));
            assert_eq!(unique.method.as_deref(), Some("btree"));
        }
        _ => assert_eq!(secondary.columns, ["price", "created"]),
    }

    // DDL is returned and mentions the table.
    let ddl = s.ddl.expect("ddl");
    assert!(ddl.contains("CREATE TABLE"), "{ddl}");
    assert!(ddl.contains(&name), "{ddl}");
}
each_engine!(structure_columns_and_indexes, sql);

async fn structure_composite_primary_key_keeps_key_order(c: &Ctx) {
    let name = c.scratch("cpk");
    let t = c.qt(&name);
    // Declared a, b, c but keyed (c, a): the key order is not the column order.
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(20)"),
    };
    c.side_exec(&format!(
        "CREATE TABLE {t} (a {int} NOT NULL, b {text}, c {text} NOT NULL, PRIMARY KEY (c, a))"
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    assert_eq!(s.primary_key, ["c", "a"]);
    assert_eq!(s.editability, Editability::Editable);
    let pk: Vec<&str> = s
        .columns
        .iter()
        .filter(|col| col.is_primary_key)
        .map(|col| col.name.as_str())
        .collect();
    assert_eq!(pk, ["a", "c"]);
    let primary = s.indexes.iter().find(|i| i.primary).expect("primary index");
    assert_eq!(primary.columns, ["c", "a"]);
    assert!(
        !s.columns
            .iter()
            .find(|c| c.name == "b")
            .unwrap()
            .is_primary_key
    );
}
each_engine!(structure_composite_primary_key_keeps_key_order, sql);

async fn structure_foreign_keys(c: &Ctx) {
    let parent = c.scratch("fk_parent");
    let other = c.scratch("fk_other");
    let child = c.scratch("fk_child");
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(20)"),
    };
    c.side_exec(&format!(
        "CREATE TABLE {} (a {int} NOT NULL, b {int} NOT NULL, PRIMARY KEY (a, b), UNIQUE (b, a))",
        c.qt(&parent)
    ))
    .await;
    c.side_exec(&format!(
        "CREATE TABLE {} (code {text} NOT NULL PRIMARY KEY)",
        c.qt(&other)
    ))
    .await;
    // The composite key is listed in a different order than the parent's columns.
    let fk1 = c.q(&c.name("fk_composite"));
    let fk2 = c.q(&c.name("fk_single"));
    c.side_exec(&format!(
        "CREATE TABLE {} (
            id {int} NOT NULL PRIMARY KEY,
            pb {int},
            pa {int},
            kind {text},
            CONSTRAINT {fk1} FOREIGN KEY (pb, pa) REFERENCES {} (b, a) ON DELETE CASCADE ON UPDATE NO ACTION,
            CONSTRAINT {fk2} FOREIGN KEY (kind) REFERENCES {} (code) ON DELETE SET NULL ON UPDATE CASCADE
        )",
        c.qt(&child),
        c.qt(&parent),
        c.qt(&other)
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&child)).await.unwrap();
    assert_eq!(s.foreign_keys.len(), 2, "{:?}", s.foreign_keys);
    let by_ref = |t: &str| {
        s.foreign_keys
            .iter()
            .find(|f| f.referenced.name == t)
            .unwrap()
    };
    let composite = by_ref(&parent);
    assert_eq!(composite.columns, ["pb", "pa"]);
    assert_eq!(composite.referenced_columns, ["b", "a"]);
    assert_eq!(composite.on_delete.as_deref(), Some("CASCADE"));
    assert_eq!(composite.on_update.as_deref(), Some("NO ACTION"));
    assert_eq!(composite.referenced.database, c.database);
    assert_eq!(composite.referenced.schema, c.schema);
    let single = by_ref(&other);
    assert_eq!(single.columns, ["kind"]);
    assert_eq!(single.referenced_columns, ["code"]);
    assert_eq!(single.on_delete.as_deref(), Some("SET NULL"));
    assert_eq!(single.on_update.as_deref(), Some("CASCADE"));
    if !c.is_sqlite() {
        assert_eq!(composite.name, c.name("fk_composite"));
        assert_eq!(single.name, c.name("fk_single"));
    }
    // The parent has no foreign keys of its own.
    let p = c.driver.table_structure(&c.tref(&parent)).await.unwrap();
    assert!(p.foreign_keys.is_empty());
}
each_engine!(structure_foreign_keys, sql);

async fn structure_editability(c: &Ctx) {
    let items = c.make_items("edit_ok", 2).await;
    assert_eq!(
        c.driver
            .table_structure(&c.tref(&items))
            .await
            .unwrap()
            .editability,
        Editability::Editable
    );

    // No primary key: read-only, with the reason.
    let no_pk = c.scratch("no_pk");
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(20)"),
    };
    c.side_exec(&format!(
        "CREATE TABLE {} (a {int}, b {text})",
        c.qt(&no_pk)
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&no_pk)).await.unwrap();
    assert!(s.primary_key.is_empty());
    assert!(s.columns.iter().all(|c| !c.is_primary_key));
    match &s.editability {
        Editability::ReadOnly(reason) => assert!(reason.contains("primary key"), "{reason}"),
        other => panic!("a table without a primary key must be read-only, got {other:?}"),
    }

    // A UNIQUE NOT NULL column is not a primary key.
    let unique_only = c.scratch("unique_only");
    c.side_exec(&format!(
        "CREATE TABLE {} (code {text} NOT NULL UNIQUE, note {text})",
        c.qt(&unique_only)
    ))
    .await;
    let s = c
        .driver
        .table_structure(&c.tref(&unique_only))
        .await
        .unwrap();
    assert!(matches!(s.editability, Editability::ReadOnly(_)));
    assert!(
        s.indexes.iter().any(|i| i.unique && i.columns == ["code"]),
        "{:?}",
        s.indexes
    );

    // Views.
    let view = c.scratch_view("view");
    c.side_exec(&format!(
        "CREATE VIEW {} AS SELECT id, name FROM {}",
        c.qt(&view),
        c.qt(&items)
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&view)).await.unwrap();
    assert_eq!(s.kind, TableKind::View);
    assert_eq!(
        s.editability,
        Editability::ReadOnly("Views can't be edited.".into())
    );
    assert_eq!(s.columns.len(), 2);
    assert!(
        s.ddl
            .as_deref()
            .is_some_and(|d| d.to_uppercase().contains("VIEW") || d.contains("SELECT"))
    );
    // Editing a read-only table is refused before anything is sent.
    let changes = dbcore::model::ChangeSet {
        table: c.tref(&view),
        changes: vec![dbcore::model::RowChange::Delete {
            key: vec![dbcore::model::ColumnValue {
                column: "id".into(),
                value: Value::Int(1),
            }],
        }],
    };
    assert!(matches!(
        c.driver.preview_changes(&changes).await,
        Err(Error::InvalidInput(m)) if m == "Views can't be edited."
    ));
    assert!(matches!(
        c.driver.apply_changes(&changes).await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(c.side_count(&items).await, 2);
}
each_engine!(structure_editability, sql);

async fn structure_of_a_missing_table_is_not_found(c: &Ctx) {
    let err = c
        .driver
        .table_structure(&c.tref("definitely_not_a_table_xyz"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    let err = c
        .driver
        .browse_page(&c.browse("definitely_not_a_table_xyz"), 0, 10)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::NotFound(_) | Error::Database { .. }),
        "{err:?}"
    );
}
each_engine!(structure_of_a_missing_table_is_not_found, sql);

async fn pg_structure_extras(c: &Ctx) {
    // Domains over enums, arrays of enums, partitions, identity vs serial.
    let ty = c.name("level");
    c.cleanup_sql(format!(
        "DROP DOMAIN IF EXISTS {}",
        c.q(&c.name("level_dom"))
    ));
    c.cleanup_sql(format!("DROP TYPE IF EXISTS {}", c.q(&ty)));
    c.side_exec(&format!("CREATE TYPE {} AS ENUM ('low', 'high')", c.q(&ty)))
        .await;
    c.side_exec(&format!(
        "CREATE DOMAIN {} AS {}",
        c.q(&c.name("level_dom")),
        c.q(&ty)
    ))
    .await;
    let name = c.scratch("pg_extra");
    c.side_exec(&format!(
        "CREATE TABLE {} (
            id serial PRIMARY KEY,
            lvl {} NOT NULL,
            lvls {}[],
            dom {},
            money_col money,
            ip inet
        )",
        c.qt(&name),
        c.q(&ty),
        c.q(&ty),
        c.q(&c.name("level_dom")),
    ))
    .await;
    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    let col = |n: &str| s.columns.iter().find(|c| c.name == n).unwrap();
    assert!(
        col("id").is_auto_increment,
        "serial counts as auto-increment"
    );
    assert_eq!(col("lvl").family, TypeFamily::Enum);
    assert_eq!(
        col("lvl").enum_values.as_deref(),
        Some(&["low".to_owned(), "high".into()][..])
    );
    assert_eq!(col("lvls").family, TypeFamily::Array);
    assert_eq!(
        col("dom").family,
        TypeFamily::Enum,
        "a domain over an enum is an enum"
    );
    assert_eq!(
        col("dom").enum_values.as_deref(),
        Some(&["low".to_owned(), "high".into()][..])
    );
    assert_eq!(col("money_col").family, TypeFamily::Decimal);
    assert_eq!(col("ip").family, TypeFamily::Other);

    // Partitioned table: the parent and a partition behave like tables.
    let parent = c.scratch("part_parent");
    let child = c.scratch("part_child");
    c.side_exec(&format!(
        "CREATE TABLE {} (id bigint, d date NOT NULL) PARTITION BY RANGE (d)",
        c.qt(&parent)
    ))
    .await;
    c.side_exec(&format!(
        "CREATE TABLE {} PARTITION OF {} FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')",
        c.qt(&child),
        c.qt(&parent)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, '2024-05-05')",
        c.qt(&parent)
    ))
    .await;
    let ps = c.driver.table_structure(&c.tref(&parent)).await.unwrap();
    assert_eq!(ps.kind, TableKind::Table);
    assert!(ps.ddl.unwrap().contains("PARTITION BY RANGE"));
    assert_eq!(c.page(&c.browse(&parent), 0, 10).await.rows.len(), 1);
    assert_eq!(c.page(&c.browse(&child), 0, 10).await.rows.len(), 1);
}
each_engine!(pg_structure_extras, postgres);

async fn sqlite_describes_tables_created_by_other_connections(c: &Ctx) {
    // Every pooled connection has loaded the schema as it is now; then another connection
    // (the editor's, another program) creates tables. The very first description of each
    // must be complete: SQLite's PRAGMAs answer from a connection's stale schema cache.
    let mut jobs = Vec::new();
    for _ in 0..5 {
        let driver = c.driver.clone();
        jobs.push(tokio::spawn(async move {
            let _ = driver.list_tables("main", None).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut cursor = driver.query(None, "SELECT 1").await.unwrap();
            while cursor.next(10).await.unwrap().is_some() {}
        }));
    }
    for job in jobs {
        job.await.unwrap();
    }
    for round in 0..6 {
        let parent = c.scratch(&format!("late_parent{round}"));
        let child = c.scratch(&format!("late_child{round}"));
        c.side_exec(&format!(
            "CREATE TABLE {} (a INTEGER NOT NULL, b INTEGER NOT NULL, PRIMARY KEY (b, a))",
            c.qt(&parent)
        ))
        .await;
        c.side_exec(&format!(
            "CREATE TABLE {} (id INTEGER PRIMARY KEY, code TEXT NOT NULL UNIQUE, pb INTEGER, pa INTEGER, \
             FOREIGN KEY (pb, pa) REFERENCES {} (b, a) ON DELETE CASCADE)",
            c.qt(&child),
            c.qt(&parent)
        ))
        .await;
        c.side_exec(&format!(
            "CREATE INDEX {} ON {} (pa)",
            c.q(&c.name(&format!("late_ix{round}"))),
            c.qt(&child)
        ))
        .await;
        c.side_exec(&format!(
            "INSERT INTO {} VALUES (3, 1), (2, 1), (1, 2), (1, 1)",
            c.qt(&parent)
        ))
        .await;
        let s = c.driver.table_structure(&c.tref(&child)).await.unwrap();
        assert_eq!(s.columns.len(), 4, "round {round}");
        let index_columns: Vec<Vec<String>> = s.indexes.iter().map(|i| i.columns.clone()).collect();
        assert!(
            index_columns.contains(&vec!["code".to_owned()]),
            "round {round}: {:?}",
            s.indexes
        );
        assert!(
            index_columns.contains(&vec!["pa".to_owned()]),
            "round {round}: {:?}",
            s.indexes
        );
        assert_eq!(
            s.foreign_keys.len(),
            1,
            "round {round}: {:?}",
            s.foreign_keys
        );
        assert_eq!(s.foreign_keys[0].columns, ["pb", "pa"]);
        assert_eq!(s.foreign_keys[0].referenced_columns, ["b", "a"]);
        // The first look at the new parent's rows already follows its primary key.
        let page = c.page(&c.browse(&parent), 0, 10).await;
        assert_eq!(
            page.rows,
            vec![
                vec![Value::Int(1), Value::Int(1)],
                vec![Value::Int(2), Value::Int(1)],
                vec![Value::Int(3), Value::Int(1)],
                vec![Value::Int(1), Value::Int(2)],
            ],
            "round {round}"
        );
        let p = c.driver.table_structure(&c.tref(&parent)).await.unwrap();
        assert_eq!(p.primary_key, ["b", "a"]);
    }
}
each_engine!(sqlite_describes_tables_created_by_other_connections, sqlite);

// ---------------------------------------------------------------------------
// Weird identifiers
// ---------------------------------------------------------------------------

/// Table and column names that need quoting everywhere.
fn weird_names(c: &Ctx) -> (String, Vec<String>) {
    // Emoji are outside the BMP, which MySQL identifiers cannot hold.
    let fancy = if c.is_my() {
        "Ωméga ünï 日本"
    } else {
        "Ωméga ünï 🎉"
    };
    let table = format!("{}_w \"q\" 'x'; --{fancy} select", c.prefix);
    let mut cols = vec![
        "id".to_owned(),
        "select".to_owned(),
        "order".to_owned(),
        "Mixed Case".to_owned(),
        "with space".to_owned(),
        "a\"b".to_owned(),
        "a'b".to_owned(),
        "semi;colon".to_owned(),
        "--dash".to_owned(),
        "100% _wild_".to_owned(),
        fancy.to_owned(),
    ];
    if c.is_my() {
        cols.push("back`tick".to_owned());
    }
    (table, cols)
}

async fn weird_identifiers_in_structure_browse_filter_sort(c: &Ctx) {
    let (table, cols) = weird_names(c);
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&table)));
    let int = if c.is_sqlite() {
        "INTEGER"
    } else if c.is_pg() {
        "integer"
    } else {
        "INT"
    };
    let text = if c.is_sqlite() {
        "TEXT"
    } else if c.is_pg() {
        "text"
    } else {
        "VARCHAR(50)"
    };
    let defs: Vec<String> = cols
        .iter()
        .map(|n| {
            if n == "id" {
                format!("{} {int} PRIMARY KEY", c.q(n))
            } else {
                format!("{} {text}", c.q(n))
            }
        })
        .collect();
    c.side_exec(&format!(
        "CREATE TABLE {} ({})",
        c.qt(&table),
        defs.join(", ")
    ))
    .await;
    let col_list: Vec<String> = cols.iter().map(|n| c.q(n)).collect();
    for i in 1..=5 {
        let vals: Vec<String> = cols
            .iter()
            .map(|n| {
                if n == "id" {
                    i.to_string()
                } else {
                    c.lit(&format!("{n}-{}", 6 - i))
                }
            })
            .collect();
        c.side_exec(&format!(
            "INSERT INTO {} ({}) VALUES ({})",
            c.qt(&table),
            col_list.join(", "),
            vals.join(", ")
        ))
        .await;
    }

    // Listed and described under the exact name.
    let tables = c
        .driver
        .list_tables(&c.database, c.schema.as_deref())
        .await
        .unwrap();
    find(&tables, &table);
    let s = c.driver.table_structure(&c.tref(&table)).await.unwrap();
    assert_eq!(s.table.name, table);
    let got: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(got, cols.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(s.primary_key, ["id"]);

    // Browse, project, sort and filter on every weird column.
    let page = c.page(&c.browse(&table), 0, 100).await;
    assert_eq!(page.rows.len(), 5);
    assert_eq!(
        page.columns
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>(),
        cols
    );
    for col in cols.iter().filter(|n| *n != "id") {
        let mut req = c.browse(&table);
        req.columns = vec!["id".into(), col.clone()];
        req.sort = vec![sort(col, false)];
        let page = c.page(&req, 0, 100).await;
        assert_eq!(
            ids(&page.rows, 0),
            [5, 4, 3, 2, 1],
            "sorted ascending by `{col}` ({}-1 < {}-5)",
            col,
            col
        );
        assert_eq!(page.columns.len(), 2, "{col}");

        let mut req = c.browse(&table);
        req.filters = vec![filter(col, FilterOp::Eq, Some(&format!("{col}-2")))];
        assert_eq!(c.page_ids(&req, 0, 100).await, [4], "filter on `{col}`");
        req.filters = vec![filter(col, FilterOp::Contains, Some("-3"))];
        assert_eq!(c.page_ids(&req, 0, 100).await, [3], "contains on `{col}`");
        assert_eq!(c.driver.count_rows(&req).await.unwrap().exact, Some(1));
    }
    // Exports and the inspector read the same names.
    assert_eq!(c.all_rows(&c.browse(&table)).await.len(), 5);
}
each_engine!(weird_identifiers_in_structure_browse_filter_sort, sql);

// ---------------------------------------------------------------------------
// Paging and sorting
// ---------------------------------------------------------------------------

async fn paging_offset_and_limit(c: &Ctx) {
    let t = c.make_items("paging", 250).await;
    let req = c.browse(&t);

    // The default order is the primary key.
    let page = c.page(&req, 0, 10).await;
    assert_eq!(page.offset, 0);
    assert_eq!(page.rows, (1..=10).map(item_row).collect::<Vec<_>>());
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "grp", "label"]);
    assert_eq!(page.columns[0].family, TypeFamily::Integer);
    assert_eq!(page.columns[1].family, TypeFamily::Text);
    assert!(
        page.query.contains(&t),
        "the page reports its SQL: {}",
        page.query
    );
    assert!(page.query.to_uppercase().contains("LIMIT 10"));

    assert_eq!(
        c.page_ids(&req, 10, 10).await,
        (11..=20).collect::<Vec<_>>()
    );
    assert_eq!(
        c.page_ids(&req, 245, 10).await,
        (246..=250).collect::<Vec<_>>()
    );
    assert_eq!(c.page_ids(&req, 249, 1).await, [250]);
    assert_eq!(c.page_ids(&req, 250, 10).await, Vec::<i64>::new());
    assert_eq!(c.page_ids(&req, 5000, 10).await, Vec::<i64>::new());
    assert_eq!(c.page_ids(&req, 0, 1000).await.len(), 250);
    let p = c.page(&req, 20, 5).await;
    assert_eq!(p.offset, 20);
    assert_eq!(ids(&p.rows, 0), [21, 22, 23, 24, 25]);

    // Past the end and limit 0 still describe the columns.
    for (offset, limit) in [(250, 10), (9999, 10), (0, 0)] {
        let p = c.page(&req, offset, limit).await;
        assert!(p.rows.is_empty());
        let names: Vec<&str> = p.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["id", "name", "grp", "label"],
            "offset {offset} limit {limit}"
        );
        assert_eq!(p.columns[0].family, TypeFamily::Integer);
    }

    // Pages tile the table exactly.
    let mut seen = Vec::new();
    for page in 0..25 {
        seen.extend(c.page_ids(&req, page * 10, 10).await);
    }
    assert_eq!(seen, (1..=250).collect::<Vec<_>>());
}
each_engine!(paging_offset_and_limit, sql);

async fn paging_an_empty_table_still_has_columns(c: &Ctx) {
    let t = c.make_items("empty", 0).await;
    let p = c.page(&c.browse(&t), 0, 50).await;
    assert!(p.rows.is_empty());
    assert_eq!(p.columns.len(), 4);
    assert_eq!(p.columns[0].name, "id");
    let count = c.driver.count_rows(&c.browse(&t)).await.unwrap();
    assert_eq!(count.exact, Some(0));
    assert!(c.all_rows(&c.browse(&t)).await.is_empty());
}
each_engine!(paging_an_empty_table_still_has_columns, sql);

async fn paging_projection_returns_only_requested_columns(c: &Ctx) {
    let t = c.make_items("proj", 30).await;
    let mut req = c.browse(&t);
    req.columns = vec!["name".into(), "id".into()];
    let p = c.page(&req, 5, 3).await;
    assert_eq!(
        p.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "id"],
        "projection keeps the requested order"
    );
    assert_eq!(
        p.rows,
        (6..=8)
            .map(|i| vec![Value::Text(item(i).name), Value::Int(i)])
            .collect::<Vec<_>>()
    );
}
each_engine!(paging_projection_returns_only_requested_columns, sql);

async fn sorting_single_and_multiple_columns(c: &Ctx) {
    let n = 200;
    let t = c.make_items("sorting", n).await;
    let mut req = c.browse(&t);

    // Descending primary key.
    req.sort = vec![sort("id", true)];
    assert_eq!(c.page_ids(&req, 0, 5).await, [200, 199, 198, 197, 196]);
    assert_eq!(c.page_ids(&req, 195, 10).await, [5, 4, 3, 2, 1]);

    // Two columns: group ascending, then id descending.
    req.sort = vec![sort("grp", false), sort("id", true)];
    let mut expected: Vec<i64> = (1..=n).collect();
    expected.sort_by_key(|&id| (item(id).grp, -id));
    assert_eq!(c.page_ids(&req, 0, 1000).await, expected);
    assert_eq!(c.page_ids(&req, 60, 40).await, expected[60..100]);

    // Three columns, mixed directions, with a text column.
    req.sort = vec![sort("grp", true), sort("name", false)];
    let mut expected: Vec<i64> = (1..=n).collect();
    expected.sort_by(|a, b| {
        item(*b)
            .grp
            .cmp(&item(*a).grp)
            .then(item(*a).name.cmp(&item(*b).name))
    });
    assert_eq!(c.page_ids(&req, 0, 1000).await, expected);

    // Text column.
    req.sort = vec![sort("name", true)];
    assert_eq!(c.page_ids(&req, 0, 3).await, [200, 199, 198]);

    // A nullable column: the NULLs end up together at one end.
    req.sort = vec![sort("label", false), sort("id", false)];
    let rows = c.page(&req, 0, 1000).await.rows;
    let labels: Vec<Option<&str>> = rows
        .iter()
        .map(|r| match &r[3] {
            Value::Text(s) => Some(s.as_str()),
            Value::Null => None,
            other => panic!("{other:?}"),
        })
        .collect();
    let nulls = labels.iter().filter(|l| l.is_none()).count();
    assert_eq!(nulls, 40);
    let (nulls_first, rest) = if labels[0].is_none() {
        (true, &labels[nulls..])
    } else {
        (false, &labels[..labels.len() - nulls])
    };
    assert!(rest.iter().all(Option::is_some));
    assert!(
        rest.windows(2).all(|w| w[0] <= w[1]),
        "the rest is ascending"
    );
    match c.engine {
        Engine::Postgres => assert!(!nulls_first, "Postgres sorts NULLs last when ascending"),
        _ => assert!(
            nulls_first,
            "{:?} sorts NULLs first when ascending",
            c.engine
        ),
    }
}
each_engine!(sorting_single_and_multiple_columns, sql);

async fn deep_paging_through_10k_rows(c: &Ctx) {
    let n = 10_000;
    let t = c.make_items("deep", n).await;
    let req = c.browse(&t);

    // Walk the table in pages of 500 and in pages of 777 (not a divisor of n).
    for size in [500u32, 777] {
        let mut all = Vec::new();
        let mut offset = 0u64;
        loop {
            let ids = c.page_ids(&req, offset, size).await;
            if ids.is_empty() {
                break;
            }
            offset += ids.len() as u64;
            all.extend(ids);
        }
        assert_eq!(all, (1..=n).collect::<Vec<_>>(), "page size {size}");
    }

    // Single pages at large offsets are exact.
    assert_eq!(
        c.page_ids(&req, 9_990, 100).await,
        (9_991..=10_000).collect::<Vec<_>>()
    );
    assert_eq!(c.page_ids(&req, 5_000, 3).await, [5_001, 5_002, 5_003]);
    assert_eq!(c.page_ids(&req, 9_999, 1).await, [10_000]);
    assert_eq!(c.page_ids(&req, 10_000, 1).await, Vec::<i64>::new());
    let mut desc = c.browse(&t);
    desc.sort = vec![sort("id", true)];
    assert_eq!(c.page_ids(&desc, 9_995, 10).await, [5, 4, 3, 2, 1]);
    assert_eq!(c.page_ids(&desc, 4_000, 2).await, [6_000, 5_999]);

    // Rows at a deep offset have the right content, not just the right ids.
    let rows = c.page(&req, 7_321, 4).await.rows;
    assert_eq!(rows, (7_322..=7_325).map(item_row).collect::<Vec<_>>());

    // Counts agree.
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!(count.exact, Some(n as u64));

    // The first page is quick, and so is a page near the end.
    for offset in [0u64, 9_900] {
        let (_, took) = c.fastest_page(&req, offset, 100).await;
        assert!(
            took < Duration::from_secs(1),
            "page at {offset} took {took:?}"
        );
    }
}
each_engine!(deep_paging_through_10k_rows, sql);

async fn pages_of_a_non_unique_sort_tile_the_table(c: &Ctx) {
    // Sorting by a column with many ties must not drop or repeat rows between pages.
    let n = 3_000;
    let t = c.make_items("ties", n).await;
    for column in ["grp", "label"] {
        for descending in [false, true] {
            let mut req = c.browse(&t);
            req.sort = vec![sort(column, descending)];
            let mut all = Vec::new();
            for page in 0..(n as u64 / 100) {
                all.extend(c.page_ids(&req, page * 100, 100).await);
            }
            let mut sorted = all.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(
                sorted,
                (1..=n).collect::<Vec<_>>(),
                "sorting by {column} (descending: {descending}) must visit every row exactly once"
            );
            assert_eq!(all.len() as i64, n);
        }
    }
}
each_engine!(pages_of_a_non_unique_sort_tile_the_table, sql);

async fn pages_of_a_table_without_a_key_tile_too(c: &Ctx) {
    // No primary key to break ties with: the pages of a sort by a column with many equal
    // values must still visit every row once (identical rows can't be told apart, so the
    // data here makes every row distinct).
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(50)"),
    };
    let t = c.scratch("nokey_ties");
    c.side_exec(&format!(
        "CREATE TABLE {} (n {int}, grp {int}, v {text})",
        c.qt(&t)
    ))
    .await;
    let rows: Vec<String> = (1..=3000)
        .map(|i| format!("({i}, {}, 'v{i}')", i % 7))
        .collect();
    for chunk in rows.chunks(500) {
        c.side_exec(&format!(
            "INSERT INTO {} VALUES {}",
            c.qt(&t),
            chunk.join(", ")
        ))
        .await;
    }
    for descending in [false, true] {
        let mut req = c.browse(&t);
        req.sort = vec![sort("grp", descending)];
        let mut all = Vec::new();
        for page in 0..30u64 {
            all.extend(c.page_ids(&req, page * 100, 100).await);
        }
        let mut seen = all.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(all.len(), 3000);
        assert_eq!(
            seen,
            (1..=3000).collect::<Vec<_>>(),
            "descending: {descending}"
        );
    }
}
each_engine!(pages_of_a_table_without_a_key_tile_too, sql);

async fn views_browse_filter_sort_and_count(c: &Ctx) {
    let t = c.make_items("view_src", 700).await;
    let v = c.scratch_view("view_browse");
    c.side_exec(&format!(
        "CREATE VIEW {} AS SELECT id, name, grp FROM {} WHERE grp <> 0",
        c.qt(&v),
        c.qt(&t)
    ))
    .await;
    let mut matview = None;
    if c.is_pg() {
        let m = c.name("matview_browse");
        c.cleanup_sql(format!("DROP MATERIALIZED VIEW IF EXISTS {}", c.qt(&m)));
        c.side_exec(&format!(
            "CREATE MATERIALIZED VIEW {} AS SELECT id, name, grp FROM {} WHERE grp <> 0",
            c.qt(&m),
            c.qt(&t)
        ))
        .await;
        matview = Some(m);
    }
    for name in std::iter::once(&v).chain(matview.as_ref()) {
        let want: Vec<i64> = (1..=700).filter(|i| item(*i).grp != 0).collect();
        let req = c.browse(name);
        let count = c.driver.count_rows(&req).await.unwrap();
        assert_eq!(count.exact, Some(want.len() as u64), "{name}");
        let page = c.page(&req, 0, 5).await;
        assert_eq!(page.columns.len(), 3);
        assert_eq!(page.rows.len(), 5);
        // Filters and sorts work on views like on tables, ties included.
        let mut filtered = c.browse(name);
        filtered.filters = vec![filter("grp", FilterOp::Eq, Some("3"))];
        filtered.sort = vec![sort("id", true)];
        let mut desc: Vec<i64> = want.iter().copied().filter(|i| item(*i).grp == 3).collect();
        desc.reverse();
        assert_eq!(c.page_ids(&filtered, 0, 1000).await, desc);
        let mut by_grp = c.browse(name);
        by_grp.sort = vec![sort("grp", false)];
        let mut all = Vec::new();
        for page in 0..(want.len() as u64).div_ceil(100) {
            all.extend(c.page_ids(&by_grp, page * 100, 100).await);
        }
        all.sort();
        assert_eq!(all, want, "{name}: a sorted view tiles too");
        assert_eq!(c.all_rows(&req).await.len(), want.len());
    }
}
each_engine!(views_browse_filter_sort_and_count, sql);

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

fn expect(n: i64, pred: impl Fn(&Item) -> bool) -> Vec<i64> {
    (1..=n).filter(|&id| pred(&item(id))).collect()
}

/// Browses with `filters` and checks both the rows and the exact count.
async fn check_filters(
    c: &Ctx,
    t: &str,
    filters: Vec<Filter>,
    match_all: bool,
    expected: Vec<i64>,
) {
    let mut req = c.browse(t);
    req.filters = filters.clone();
    req.match_all = match_all;
    let got = c.page_ids(&req, 0, 5000).await;
    assert_eq!(got, expected, "filters {filters:?} (match_all {match_all})");
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!(
        count.exact,
        Some(expected.len() as u64),
        "count for {filters:?}"
    );
    assert_eq!(count.estimate, None, "filtered counts have no estimate");
    // The export path agrees with the grid.
    let all = ids(&c.all_rows(&req).await, 0);
    let mut all_sorted = all.clone();
    all_sorted.sort();
    assert_eq!(all_sorted, expected, "browse_all for {filters:?}");
}

async fn filters_comparison_operators(c: &Ctx) {
    let n = 100;
    let t = c.make_items("flt_cmp", n).await;
    use FilterOp::*;
    check_filters(
        c,
        &t,
        vec![filter("grp", Eq, Some("3"))],
        true,
        expect(n, |i| i.grp == 3),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("grp", Neq, Some("3"))],
        true,
        expect(n, |i| i.grp != 3),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("grp", Lt, Some("2"))],
        true,
        expect(n, |i| i.grp < 2),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("grp", Lte, Some("2"))],
        true,
        expect(n, |i| i.grp <= 2),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("grp", Gt, Some("5"))],
        true,
        expect(n, |i| i.grp > 5),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("grp", Gte, Some("5"))],
        true,
        expect(n, |i| i.grp >= 5),
    )
    .await;
    // Numbers compare as numbers, not as text ("9" > "10" as text).
    check_filters(
        c,
        &t,
        vec![filter("id", Gt, Some("9"))],
        true,
        expect(n, |i| i.id > 9),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("id", Lt, Some("10"))],
        true,
        expect(n, |i| i.id < 10),
    )
    .await;
    // Text compares as text.
    check_filters(
        c,
        &t,
        vec![filter("name", Lt, Some("item 00010"))],
        true,
        expect(n, |i| i.id < 10),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", Gte, Some("item 00095"))],
        true,
        expect(n, |i| i.id >= 95),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", Eq, Some("item 00042"))],
        true,
        vec![42],
    )
    .await;
    // Equality on a nullable column does not match the NULL rows (SQL semantics), and
    // neither does inequality.
    check_filters(
        c,
        &t,
        vec![filter("label", Eq, Some("lbl-1"))],
        true,
        expect(n, |i| i.label.as_deref() == Some("lbl-1")),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("label", Neq, Some("lbl-1"))],
        true,
        expect(n, |i| i.label.as_deref().is_some_and(|l| l != "lbl-1")),
    )
    .await;
    // Nothing matches.
    check_filters(c, &t, vec![filter("grp", Eq, Some("99"))], true, vec![]).await;
}
each_engine!(filters_comparison_operators, sql);

async fn filters_text_operators(c: &Ctx) {
    let n = 100;
    let t = c.make_items("flt_txt", n).await;
    use FilterOp::*;
    check_filters(
        c,
        &t,
        vec![filter("name", Contains, Some("tem 0001"))],
        true,
        expect(n, |i| (10..=19).contains(&i.id)),
    )
    .await;
    // Case-insensitive.
    check_filters(
        c,
        &t,
        vec![filter("name", Contains, Some("ITEM 0001"))],
        true,
        expect(n, |i| (10..=19).contains(&i.id)),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("label", NotContains, Some("lbl-1"))],
        true,
        expect(n, |i| {
            i.label.as_deref().is_some_and(|l| !l.contains("lbl-1"))
        }),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", StartsWith, Some("item 0005"))],
        true,
        expect(n, |i| (50..=59).contains(&i.id)),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", StartsWith, Some("ITEM 0005"))],
        true,
        expect(n, |i| (50..=59).contains(&i.id)),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", EndsWith, Some("7"))],
        true,
        expect(n, |i| i.name.ends_with('7')),
    )
    .await;
    // Raw LIKE patterns: `_` is one character, `%` any run.
    check_filters(
        c,
        &t,
        vec![filter("name", Like, Some("item 000_5"))],
        true,
        expect(n, |i| i.id % 10 == 5 && i.id < 100),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("label", Like, Some("lbl-1%"))],
        true,
        expect(n, |i| {
            i.label.as_deref().is_some_and(|l| l.starts_with("lbl-1"))
        }),
    )
    .await;
    // LIKE on a number column compares its text.
    check_filters(
        c,
        &t,
        vec![filter("id", Contains, Some("9"))],
        true,
        expect(n, |i| i.id.to_string().contains('9')),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("id", StartsWith, Some("1"))],
        true,
        expect(n, |i| i.id.to_string().starts_with('1')),
    )
    .await;
}
each_engine!(filters_text_operators, sql);

async fn filters_in_and_null_checks(c: &Ctx) {
    let n = 100;
    let t = c.make_items("flt_in", n).await;
    use FilterOp::*;
    check_filters(
        c,
        &t,
        vec![filter("id", In, Some("1,2,3"))],
        true,
        vec![1, 2, 3],
    )
    .await;
    // Whitespace around items is trimmed, duplicates collapse.
    check_filters(
        c,
        &t,
        vec![filter("id", In, Some(" 3 , 1,3 , 2 "))],
        true,
        vec![1, 2, 3],
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("id", NotIn, Some("1,2,3"))],
        true,
        (4..=n).collect(),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", In, Some("item 00001,item 00002,nope"))],
        true,
        vec![1, 2],
    )
    .await;
    // Quoted items keep commas and whitespace.
    check_filters(
        c,
        &t,
        vec![filter("name", In, Some("'item 00007', 'item 00008'"))],
        true,
        vec![7, 8],
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("name", In, Some("'a,b','item 00009'"))],
        true,
        vec![9],
    )
    .await;
    // An empty list matches nothing; NOT IN of nothing matches everything.
    check_filters(c, &t, vec![filter("id", In, Some(""))], true, vec![]).await;
    check_filters(
        c,
        &t,
        vec![filter("id", NotIn, Some(""))],
        true,
        (1..=n).collect(),
    )
    .await;

    // NULL checks ignore the value.
    check_filters(
        c,
        &t,
        vec![filter("label", IsNull, None)],
        true,
        expect(n, |i| i.label.is_none()),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("label", IsNull, Some("ignored"))],
        true,
        expect(n, |i| i.label.is_none()),
    )
    .await;
    check_filters(
        c,
        &t,
        vec![filter("label", IsNotNull, None)],
        true,
        expect(n, |i| i.label.is_some()),
    )
    .await;
    check_filters(c, &t, vec![filter("id", IsNull, None)], true, vec![]).await;
    check_filters(
        c,
        &t,
        vec![filter("id", IsNotNull, None)],
        true,
        (1..=n).collect(),
    )
    .await;
}
each_engine!(filters_in_and_null_checks, sql);

async fn filters_combine_with_and_or_and_raw_where(c: &Ctx) {
    let n = 100;
    let t = c.make_items("flt_comb", n).await;
    use FilterOp::*;
    // AND (the default).
    check_filters(
        c,
        &t,
        vec![filter("grp", Eq, Some("1")), filter("label", IsNull, None)],
        true,
        expect(n, |i| i.grp == 1 && i.label.is_none()),
    )
    .await;
    // OR.
    check_filters(
        c,
        &t,
        vec![filter("grp", Eq, Some("1")), filter("label", IsNull, None)],
        false,
        expect(n, |i| i.grp == 1 || i.label.is_none()),
    )
    .await;
    // Three filters.
    check_filters(
        c,
        &t,
        vec![
            filter("grp", Gte, Some("2")),
            filter("id", Lt, Some("50")),
            filter("name", Contains, Some("3")),
        ],
        true,
        expect(n, |i| i.grp >= 2 && i.id < 50 && i.name.contains('3')),
    )
    .await;

    // Raw WHERE, with and without the keyword and with a trailing comment.
    let expected = expect(n, |i| i.grp == 3 && i.label.is_some());
    for raw in [
        "grp = 3 AND label IS NOT NULL",
        "WHERE grp = 3 AND label IS NOT NULL",
        "  where  grp = 3 AND label IS NOT NULL  ",
        "grp = 3 AND label IS NOT NULL -- trailing comment",
        "(grp = 3) AND (label IS NOT NULL)",
    ] {
        let mut req = c.browse(&t);
        req.raw_where = Some(raw.to_owned());
        assert_eq!(c.page_ids(&req, 0, 1000).await, expected, "raw: {raw:?}");
        assert_eq!(
            c.driver.count_rows(&req).await.unwrap().exact,
            Some(expected.len() as u64)
        );
    }
    // Raw WHERE with OR is parenthesised before it is ANDed with the filters.
    let mut req = c.browse(&t);
    req.raw_where = Some("grp = 1 OR grp = 2".into());
    req.filters = vec![filter("id", Lt, Some("20"))];
    assert_eq!(
        c.page_ids(&req, 0, 1000).await,
        expect(n, |i| (i.grp == 1 || i.grp == 2) && i.id < 20)
    );
    // Raw WHERE together with OR-ed filters.
    req.match_all = false;
    req.raw_where = Some("id > 90".into());
    req.filters = vec![filter("grp", Eq, Some("0")), filter("grp", Eq, Some("1"))];
    assert_eq!(
        c.page_ids(&req, 0, 1000).await,
        expect(n, |i| (i.grp == 0 || i.grp == 1) && i.id > 90)
    );
    // Sorting and paging apply on top.
    let mut req = c.browse(&t);
    req.raw_where = Some("grp = 3".into());
    req.sort = vec![sort("id", true)];
    let all = expect(n, |i| i.grp == 3);
    let mut desc = all.clone();
    desc.reverse();
    assert_eq!(c.page_ids(&req, 0, 4).await, desc[..4]);
    assert_eq!(c.page_ids(&req, 4, 4).await, desc[4..8]);
    // Always false / always true.
    req.raw_where = Some("1 = 0".into());
    assert!(c.page_ids(&req, 0, 10).await.is_empty());
    assert_eq!(c.driver.count_rows(&req).await.unwrap().exact, Some(0));
    req.raw_where = Some("   ".into());
    assert_eq!(
        c.page_ids(&req, 0, 1000).await.len(),
        n as usize,
        "blank raw WHERE is ignored"
    );
    req.raw_where = Some("WHERE".into());
    assert!(
        c.driver.browse_page(&req, 0, 5).await.is_err()
            || c.page_ids(&req, 0, 1000).await.len() == n as usize
    );
}
each_engine!(filters_combine_with_and_or_and_raw_where, sql);

async fn filter_errors_are_reported(c: &Ctx) {
    let t = c.make_items("flt_err", 10).await;
    use FilterOp::*;
    // A value-taking operator without a value.
    for op in [
        Eq,
        Neq,
        Lt,
        Lte,
        Gt,
        Gte,
        Contains,
        NotContains,
        StartsWith,
        EndsWith,
        Like,
        In,
        NotIn,
    ] {
        let mut req = c.browse(&t);
        req.filters = vec![filter("id", op, None)];
        let err = c.driver.browse_page(&req, 0, 5).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{op:?}: {err:?}");
        let err = c.driver.count_rows(&req).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "{op:?} count: {err:?}"
        );
        assert!(
            c.driver.browse_all(&req).await.is_err(),
            "{op:?} browse_all"
        );
    }
    // Broken raw WHERE: the server's error comes back with its code.
    let mut req = c.browse(&t);
    req.raw_where = Some("id = = 1".into());
    let err = c.driver.browse_page(&req, 0, 5).await.unwrap_err();
    assert!(
        matches!(err, Error::Database { code: Some(_), .. }),
        "{err:?}"
    );
    assert!(c.driver.count_rows(&req).await.is_err());
    req.raw_where = Some("id = 1)".into());
    assert!(c.driver.browse_page(&req, 0, 5).await.is_err());
    // A comment in the raw clause cannot swallow the closing parenthesis.
    req.raw_where = Some("id = 1 -- and then some".into());
    assert_eq!(c.page_ids(&req, 0, 5).await, [1]);
    // The connection is still good.
    assert_eq!(c.page_ids(&c.browse(&t), 0, 3).await, [1, 2, 3]);
}
each_engine!(filter_errors_are_reported, sql);

/// Naming a column the table lacks is an error for filters, sorts and projections.
async fn unknown_columns_are_errors(c: &Ctx) {
    let t = c.make_items("unknown_col", 10).await;
    let mut req = c.browse(&t);
    req.filters = vec![filter("no_such_column", FilterOp::Eq, Some("1"))];
    let err = c.driver.browse_page(&req, 0, 5).await.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "filter: {err:?}");
    assert!(c.driver.count_rows(&req).await.is_err(), "filter count");

    let mut req = c.browse(&t);
    req.sort = vec![sort("no_such_column", false)];
    let err = c.driver.browse_page(&req, 0, 5).await.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "sort: {err:?}");

    let mut req = c.browse(&t);
    req.columns = vec!["no_such_column".into()];
    let err = c.driver.browse_page(&req, 0, 5).await.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "projection: {err:?}");
}
each_engine!(unknown_columns_are_errors, postgres, mysql, mariadb);

/// SQLite reads an unknown double-quoted identifier as a string literal (its "DQS"
/// misfeature), so the same requests silently succeed there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "BUG: SQLite treats a missing double-quoted column as a string literal, so a filter, sort or projection on it silently succeeds instead of failing (sqlite.rs/sqlcommon.rs select_sql)"]
async fn unknown_columns_are_errors_sqlite() {
    run_on(Engine::Sqlite, unknown_columns_are_errors).await
}

async fn filter_values_are_data_not_sql(c: &Ctx) {
    // Values with quotes, wildcards, backslashes and SQL-looking text must be matched literally.
    let name = c.scratch("special");
    let t = c.qt(&name);
    let text = if c.is_pg() {
        "text"
    } else if c.is_sqlite() {
        "TEXT"
    } else {
        "VARCHAR(100)"
    };
    let int = if c.is_pg() {
        "integer"
    } else if c.is_sqlite() {
        "INTEGER"
    } else {
        "INT"
    };
    c.side_exec(&format!(
        "CREATE TABLE {t} (id {int} PRIMARY KEY, v {text})"
    ))
    .await;
    let values = [
        "100%",
        "50% off",
        "a_b",
        "axb",
        "c\\d",
        "back\\\\slash",
        "O'Brien",
        "say \"hi\"",
        "semi;colon",
        "a,b",
        "x' OR '1'='1",
        "'; DROP TABLE t;--",
        "ünï çödé",
        "plain",
        "",
    ];
    for (i, v) in values.iter().enumerate() {
        c.side_exec(&format!("INSERT INTO {t} VALUES ({}, {})", i + 1, c.lit(v)))
            .await;
    }
    let find_ids = |f: Filter| {
        let mut req = c.browse(&name);
        req.filters = vec![f];
        async move { c.page_ids(&req, 0, 100).await }
    };
    let id_of = |v: &str| values.iter().position(|x| *x == v).unwrap() as i64 + 1;
    use FilterOp::*;
    for v in values.iter().filter(|v| !v.is_empty()) {
        assert_eq!(
            find_ids(filter("v", Eq, Some(v))).await,
            [id_of(v)],
            "Eq {v:?}"
        );
    }
    assert_eq!(
        find_ids(filter("v", Eq, Some(""))).await,
        [id_of("")],
        "empty string is not NULL"
    );
    assert_eq!(find_ids(filter("v", IsNull, None)).await, Vec::<i64>::new());
    // Wildcards in the search text are literal for contains / starts with / ends with.
    assert_eq!(
        find_ids(filter("v", Contains, Some("%"))).await,
        [id_of("100%"), id_of("50% off")]
    );
    assert_eq!(
        find_ids(filter("v", Contains, Some("_"))).await,
        [id_of("a_b")]
    );
    assert_eq!(
        find_ids(filter("v", StartsWith, Some("a_"))).await,
        [id_of("a_b")]
    );
    assert_eq!(
        find_ids(filter("v", EndsWith, Some("%"))).await,
        [id_of("100%")]
    );
    assert_eq!(
        find_ids(filter("v", Contains, Some("\\"))).await.len(),
        2,
        "backslashes"
    );
    assert_eq!(
        find_ids(filter("v", Contains, Some("c\\d"))).await,
        [id_of("c\\d")]
    );
    assert_eq!(
        find_ids(filter("v", Contains, Some("'"))).await.len(),
        3,
        "single quotes"
    );
    assert_eq!(
        find_ids(filter("v", Contains, Some("\""))).await,
        [id_of("say \"hi\"")]
    );
    assert_eq!(find_ids(filter("v", Contains, Some(";"))).await.len(), 2);
    assert_eq!(
        find_ids(filter("v", Contains, Some("ünï"))).await,
        [id_of("ünï çödé")]
    );
    // A raw LIKE pattern keeps its wildcards.
    assert_eq!(
        find_ids(filter("v", Like, Some("a_b"))).await,
        [id_of("a_b"), id_of("axb"), id_of("a,b")]
    );
    // IN with quoted items containing commas and quotes.
    assert_eq!(
        find_ids(filter("v", In, Some("'a,b', plain"))).await,
        [id_of("a,b"), id_of("plain")]
    );
    assert_eq!(
        find_ids(filter("v", In, Some("'O''Brien'"))).await,
        [id_of("O'Brien")]
    );
    // Injection attempts are inert and the table is still there.
    assert_eq!(
        find_ids(filter("v", Eq, Some("x' OR '1'='1"))).await,
        [id_of("x' OR '1'='1")]
    );
    assert!(
        find_ids(filter("v", Eq, Some("' OR 1=1 --")))
            .await
            .is_empty()
    );
    assert!(
        find_ids(filter("v", Contains, Some("'; DROP TABLE")))
            .await
            .len()
            == 1
    );
    assert_eq!(c.side_count(&name).await, values.len() as i64);
    // NOT CONTAINS keeps everything else.
    assert_eq!(
        find_ids(filter("v", NotContains, Some("a"))).await.len(),
        values
            .iter()
            .filter(|v| !v.to_lowercase().contains('a'))
            .count()
    );
}
each_engine!(filter_values_are_data_not_sql, sql);

// ---------------------------------------------------------------------------
// Counts
// ---------------------------------------------------------------------------

async fn counts_exact_and_estimate(c: &Ctx) {
    let t = c.make_items("counts", 1234).await;
    let req = c.browse(&t);
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!(count.exact, Some(1234), "small tables are counted exactly");
    if c.is_sqlite() {
        assert_eq!(count.estimate, None);
    }
    // Filtered counts are exact and carry no estimate.
    let mut filtered = c.browse(&t);
    filtered.filters = vec![filter("grp", FilterOp::Eq, Some("2"))];
    let count = c.driver.count_rows(&filtered).await.unwrap();
    assert_eq!(count.exact, Some(expect(1234, |i| i.grp == 2).len() as u64));
    assert_eq!(count.estimate, None);

    if c.is_pg() {
        // After ANALYZE the planner statistics know the size.
        c.side_exec(&format!("ANALYZE {}", c.qt(&t))).await;
        let count = c.driver.count_rows(&req).await.unwrap();
        assert_eq!(count.exact, Some(1234));
        assert_eq!(count.estimate, Some(1234));
    }
    // The count follows changes immediately.
    c.side_exec(&format!("DELETE FROM {} WHERE id <= 234", c.qt(&t)))
        .await;
    assert_eq!(c.driver.count_rows(&req).await.unwrap().exact, Some(1000));
}
each_engine!(counts_exact_and_estimate, sql);

async fn counts_of_the_seeded_million_row_table(c: &Ctx) {
    // Read-only. Above one million estimated rows only the estimate is returned.
    let req = c.browse("big_table");
    let started = Instant::now();
    let count = c.driver.count_rows(&req).await.unwrap();
    let took = started.elapsed();
    match count.exact {
        Some(n) => assert_eq!(n, 1_000_000),
        None => {
            let est = count.estimate.expect("estimate when not exact");
            assert!((1_000_000..5_000_000).contains(&est), "{count:?}");
        }
    }
    assert!(took < Duration::from_secs(5), "count took {took:?}");

    // Paging the big table: first page and a deep one stay fast.
    let (page, took) = c.fastest_page(&req, 0, 100).await;
    assert!(took < Duration::from_secs(1), "first page took {took:?}");
    assert_eq!(page.rows.len(), 100);
    assert_eq!(page.rows[0][0], Value::Int(1));
    let started = Instant::now();
    let page = c.page(&req, 500_000, 100).await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "page at 500000 took {:?}",
        started.elapsed()
    );
    assert_eq!(page.rows[0][0], Value::Int(500_001));
    let mut desc = req.clone();
    desc.sort = vec![sort("id", true)];
    let page = c.page(&desc, 0, 3).await;
    assert_eq!(ids(&page.rows, 0), [1_000_000, 999_999, 999_998]);
    // A selective filter on a big table.
    let mut filtered = req.clone();
    filtered.filters = vec![filter("id", FilterOp::Gt, Some("999990"))];
    assert_eq!(
        c.driver.count_rows(&filtered).await.unwrap().exact,
        Some(10)
    );
}
each_engine!(
    counts_of_the_seeded_million_row_table,
    postgres,
    mysql,
    mariadb
);

async fn sqlite_first_page_of_a_large_table_is_quick(c: &Ctx) {
    let t = c.scratch("big");
    c.side_exec(&format!(
        "CREATE TABLE {} (id INTEGER PRIMARY KEY, name TEXT)",
        c.qt(&t)
    ))
    .await;
    c.side_exec(&format!(
        "WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < 200000) \
         INSERT INTO {} SELECT i, 'row ' || i FROM s",
        c.qt(&t)
    ))
    .await;
    let req = c.browse(&t);
    let (page, took) = c.fastest_page(&req, 0, 100).await;
    assert!(took < Duration::from_secs(1), "first page took {took:?}");
    assert_eq!(page.rows.len(), 100);
    let started = Instant::now();
    let page = c.page(&req, 150_000, 100).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(page.rows[0][0], Value::Int(150_001));
    assert_eq!(
        c.driver.count_rows(&req).await.unwrap().exact,
        Some(200_000)
    );
}
each_engine!(sqlite_first_page_of_a_large_table_is_quick, sqlite);

// ---------------------------------------------------------------------------
// The whole nasty schema
// ---------------------------------------------------------------------------

async fn every_seeded_table_describes_and_browses(c: &Ctx) {
    if c.is_sqlite() {
        // Same schema as the docker servers, loaded into this test's own file.
        c.side_exec(include_str!("../../../testdb/sqlite/nasty.sql"))
            .await;
    }
    let schemas: Vec<Option<String>> = if c.is_pg() {
        c.driver
            .list_schemas(DB)
            .await
            .unwrap()
            .into_iter()
            .map(Some)
            .collect()
    } else {
        vec![c.schema.clone()]
    };
    let mut checked = 0;
    for schema in schemas {
        let tables = c
            .driver
            .list_tables(&c.database, schema.as_deref())
            .await
            .unwrap();
        for info in tables.iter().filter(|t| !t.table.name.starts_with("tst_")) {
            let table = TableRef {
                database: c.database.clone(),
                schema: schema.clone(),
                name: info.table.name.clone(),
            };
            let what = format!("{schema:?}.{}", table.name);
            let structure = c
                .driver
                .table_structure(&table)
                .await
                .unwrap_or_else(|e| panic!("structure of {what}: {e:?}"));
            assert!(!structure.columns.is_empty(), "{what}");
            assert_eq!(structure.kind, info.kind, "{what}");
            assert!(structure.ddl.is_some(), "ddl of {what}");
            let mut request = c.browse(&table.name);
            request.table = table.clone();
            let page = c
                .driver
                .browse_page(&request, 0, 25)
                .await
                .unwrap_or_else(|e| panic!("browse {what}: {e:?}"));
            assert_eq!(page.columns.len(), structure.columns.len(), "{what}");
            assert!(
                page.rows.iter().all(|r| r.len() == page.columns.len()),
                "{what}"
            );
            c.driver
                .count_rows(&request)
                .await
                .unwrap_or_else(|e| panic!("count {what}: {e:?}"));
            // The export stream opens, delivers its columns and can be dropped.
            let mut cursor = c.driver.browse_all(&request).await.unwrap();
            let _ = cursor
                .next(10)
                .await
                .unwrap_or_else(|e| panic!("export of {what}: {e:?}"));
            checked += 1;
        }
    }
    assert!(checked >= 12, "only {checked} tables checked");
    if c.is_sqlite() {
        let tables = c.driver.list_tables("main", None).await.unwrap();
        for name in [
            "all_types",
            "big_table",
            "no_pk",
            "composite_pk",
            "select",
            "empty_table",
            "strict_table",
            "without_rowid",
            "docs_fts",
        ] {
            find(&tables, name);
        }
        assert_eq!(find(&tables, "docs_fts").kind, TableKind::Virtual);
        assert!(
            !tables.iter().any(|t| t.table.name.starts_with("docs_fts_")),
            "FTS shadow tables are hidden"
        );
    }
}
each_engine!(every_seeded_table_describes_and_browses, sql);

async fn parallel_browsing_returns_the_right_pages(c: &Ctx) {
    let n = 2000;
    let t = c.make_items("parallel", n).await;
    let mut jobs = Vec::new();
    for i in 0..60u64 {
        let driver = c.driver.clone();
        let req = c.browse(&t);
        jobs.push(tokio::spawn(async move {
            let offset = (i * 31) % 1900;
            let page = driver.browse_page(&req, offset, 50).await?;
            Ok::<_, Error>((offset, ids(&page.rows, 0)))
        }));
    }
    for job in jobs {
        let (offset, got) = job.await.unwrap().unwrap();
        assert_eq!(
            got,
            ((offset as i64 + 1)..=(offset as i64 + 50)).collect::<Vec<_>>(),
            "page at {offset}"
        );
    }
    // Counts and structure requests mixed in.
    let (count_req, page_req, table) = (c.browse(&t), c.browse(&t), c.tref(&t));
    let (a, b, d) = tokio::join!(
        c.driver.count_rows(&count_req),
        c.driver.table_structure(&table),
        c.driver.browse_page(&page_req, 1990, 50)
    );
    assert_eq!(a.unwrap().exact, Some(n as u64));
    assert_eq!(b.unwrap().columns.len(), 4);
    assert_eq!(d.unwrap().rows.len(), 10);
}
each_engine!(parallel_browsing_returns_the_right_pages, sql);
