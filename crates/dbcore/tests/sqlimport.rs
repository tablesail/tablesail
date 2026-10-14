//! Importing a .sql file (a dump) against every SQL engine: statement splitting as each engine's
//! dumps need it, error handling, session state between statements, transactions, read-only
//! connections and cancelling. The streaming and dialect details of the reader have unit tests
//! next to the code; these run the real servers.

#[macro_use]
mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use dbcore::export::{CancellationToken, ProgressFn};
use dbcore::model::{JobProgress, SqlImport};
use dbcore::querylog;
use dbcore::sqlimport::{check, run_sql_import};
use dbcore::{Engine, Error, Value};

/// A scratch directory removed at the end of the test.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!("tablesail-it-sql-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// Writes `contents` to a file and returns its path.
    fn file(&self, name: &str, contents: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn spec(c: &Ctx, path: String, stop_on_error: bool) -> SqlImport {
    SqlImport {
        path,
        database: Some(c.database.clone()),
        stop_on_error,
    }
}

/// Imports `sql` and returns the final progress (also checking the last report equals it).
async fn import_with(c: &Ctx, sql: &str, stop_on_error: bool, read_only: bool) -> JobProgress {
    let dir = TempDir::new();
    let path = dir.file("dump.sql", sql.as_bytes());
    let seen = Arc::new(Mutex::new(Vec::<JobProgress>::new()));
    let sink = seen.clone();
    let progress: ProgressFn = Arc::new(move |p| sink.lock().unwrap().push(p.clone()));
    let done = run_sql_import(
        c.driver.clone(),
        spec(c, path, stop_on_error),
        read_only,
        progress,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(done.done, "{done:?}");
    assert_eq!(
        seen.lock().unwrap().last(),
        Some(&done),
        "the last report is the result"
    );
    done
}

async fn import(c: &Ctx, sql: &str) -> JobProgress {
    import_with(c, sql, true, false).await
}

/// (integer, text) column types of the engine.
fn types(c: &Ctx) -> (&'static str, &'static str) {
    match c.engine {
        Engine::Sqlite => ("INTEGER", "TEXT"),
        Engine::Postgres => ("integer", "text"),
        _ => ("INT", "VARCHAR(200)"),
    }
}

// ---------------------------------------------------------------------------
// A plain dump
// ---------------------------------------------------------------------------

async fn imports_a_dump(c: &Ctx) {
    let t = c.scratch("dump");
    let v = c.scratch_view("dump_v");
    let (qt, qv, qi) = (c.qt(&t), c.qt(&v), c.q(&c.name("dump_ix")));
    let (int, text) = types(c);
    let dump = format!(
        "-- {t}: a hand-made dump\n\
         CREATE TABLE {qt} (id {int} PRIMARY KEY, name {text} NOT NULL, note {text});\n\
         INSERT INTO {qt} (id, name, note) VALUES (1, 'plain', NULL), \
         (2, 'O''Brien; \"quoted\"', 'semi;colon -- not a comment'), (3, 'ünï ✓ 日本', '');\n\
         /* block comment; with semicolon */\n\
         INSERT INTO {qt} (id, name) VALUES (4, 'four');\n\
         CREATE INDEX {qi} ON {qt} (name);\n\
         CREATE VIEW {qv} AS SELECT id, name FROM {qt} WHERE id > 1;\n\
         UPDATE {qt} SET note = 'updated' WHERE id = 4;\n\
         DELETE FROM {qt} WHERE id = 1;\n\
         SELECT count(*) FROM {qt};\n"
    );
    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    let done = import(c, &dump).await;

    assert_eq!(done.error, None);
    assert_eq!(done.warning, None);
    assert!(!done.cancelled);
    assert_eq!(done.rows, 8, "eight statements ran");
    assert_eq!(done.bytes, dump.len() as u64, "the whole file was consumed");

    assert_eq!(c.side_count(&t).await, 3);
    assert_eq!(
        c.side_column(&format!("SELECT name FROM {qt} ORDER BY id"))
            .await,
        [
            Some("O'Brien; \"quoted\"".to_owned()),
            Some("ünï ✓ 日本".to_owned()),
            Some("four".to_owned())
        ]
    );
    assert_eq!(
        c.side_column(&format!("SELECT note FROM {qt} ORDER BY id"))
            .await,
        [
            Some("semi;colon -- not a comment".to_owned()),
            Some(String::new()),
            Some("updated".to_owned())
        ]
    );
    assert_eq!(
        c.side_text(&format!("SELECT count(*) FROM {qv}"))
            .await
            .as_deref(),
        Some("3")
    );
    // The statements went through the driver, so the console shows them.
    assert!(
        querylog::since(seq)
            .iter()
            .any(|e| e.sql.contains("O''Brien") && e.sql.contains(&t)),
        "the INSERT is in the query log"
    );
}
each_engine!(imports_a_dump, sql);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

async fn stops_at_the_first_failed_statement(c: &Ctx) {
    let t = c.scratch("stop");
    let missing = c.name("missing");
    let (qt, qm) = (c.qt(&t), c.qt(&missing));
    let (int, _) = types(c);
    let dump = format!(
        "CREATE TABLE {qt} (a {int});\n\
         INSERT INTO {qt} VALUES (1);\n\
         INSERT INTO {qm} VALUES (2);\n\
         INSERT INTO {qt} VALUES (3);\n"
    );
    let done = import(c, &dump).await;
    let error = done.error.expect("the job failed");
    assert!(error.starts_with("Statement 3 failed: "), "{error}");
    assert!(
        error.contains(&missing),
        "the server's message is kept: {error}"
    );
    assert_eq!(done.rows, 2, "two statements ran");
    assert!(!done.cancelled);
    // Nothing after the failure ran, and what ran before it stays.
    assert_eq!(c.side_count(&t).await, 1);
    // The connection is fine afterwards.
    assert_eq!(
        c.scalar(&format!("SELECT count(*) FROM {qt}")).await,
        Value::Int(1)
    );
}
each_engine!(stops_at_the_first_failed_statement, sql);

async fn can_carry_on_after_failed_statements(c: &Ctx) {
    let t = c.scratch("carry");
    let missing = c.name("missing");
    let (qt, qm) = (c.qt(&t), c.qt(&missing));
    let (int, _) = types(c);
    let dump = format!(
        "CREATE TABLE {qt} (a {int} PRIMARY KEY);\n\
         INSERT INTO {qm} VALUES (1);\n\
         INSERT INTO {qt} VALUES (2);\n\
         INSERT INTO {qt} VALUES (2);\n\
         THIS IS NOT SQL;\n\
         INSERT INTO {qt} VALUES (3);\n"
    );
    let done = import_with(c, &dump, false, false).await;
    assert_eq!(done.error, None);
    assert_eq!(done.rows, 3, "the table and two rows");
    let warning = done.warning.expect("failures are reported as a warning");
    assert!(
        warning.starts_with("3 of 6 statements failed; first: statement 2: "),
        "{warning}"
    );
    assert!(warning.contains(&missing), "{warning}");
    assert_eq!(done.bytes, dump.len() as u64);
    assert_eq!(
        c.side_column(&format!("SELECT a FROM {qt} ORDER BY a"))
            .await,
        [Some("2".to_owned()), Some("3".to_owned())]
    );
}
each_engine!(can_carry_on_after_failed_statements, sql);

async fn a_file_without_statements_says_so(c: &Ctx) {
    for sql in ["", "\n-- only a comment\n/* and another */\n", "\u{feff}"] {
        let done = import(c, sql).await;
        assert_eq!((done.rows, &done.error), (0, &None));
        assert_eq!(
            done.warning.as_deref(),
            Some("The file contains no SQL statements.")
        );
    }
}
each_engine!(a_file_without_statements_says_so, sql);

async fn unusable_files_are_reported(c: &Ctx) {
    let dir = TempDir::new();
    let go = |path: String| {
        run_sql_import(
            c.driver.clone(),
            spec(c, path, true),
            false,
            Arc::new(|_| {}),
            CancellationToken::new(),
        )
    };
    // Before anything runs: no such file, not a file, no path.
    let missing = dir.0.join("missing.sql").to_string_lossy().into_owned();
    assert!(matches!(go(missing.clone()).await, Err(Error::Io(_))));
    assert!(matches!(
        check(c.engine, &spec(c, missing, true)).await,
        Err(Error::Io(_))
    ));
    assert!(matches!(
        go(dir.0.to_string_lossy().into_owned()).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        go(String::new()).await,
        Err(Error::InvalidInput(_))
    ));

    // In the file: not UTF-8 (a Latin-1 dump) ends the job and says where.
    let latin1 = dir.file("latin1.sql", b"SELECT 1;\nSELECT 'caf\xE9';\n");
    let done = go(latin1).await.unwrap();
    let error = done.error.expect("an error");
    assert!(error.contains("isn't valid UTF-8"), "{error}");
    assert!(error.contains("position 21"), "{error}");
    assert!(done.done);
}
each_engine!(unusable_files_are_reported, sql);

// ---------------------------------------------------------------------------
// Sessions, transactions, read-only
// ---------------------------------------------------------------------------

async fn session_state_lasts_through_the_file(c: &Ctx) {
    let temp = c.name("scratch");
    let copy = c.scratch("copied");
    let (qtemp, qcopy) = (c.q(&temp), c.qt(&copy));
    let (int, _) = types(c);
    let create_temp = if c.is_my() {
        "CREATE TEMPORARY TABLE"
    } else {
        "CREATE TEMP TABLE"
    };
    // A temporary table exists for one connection only: the later statements must find it.
    let dump = format!(
        "{create_temp} {qtemp} (a {int});\n\
         INSERT INTO {qtemp} VALUES (5), (6), (7);\n\
         CREATE TABLE {qcopy} AS SELECT a FROM {qtemp};\n"
    );
    let done = import(c, &dump).await;
    assert_eq!((done.rows, &done.error), (3, &None));
    assert_eq!(c.side_count(&copy).await, 3);
}
each_engine!(session_state_lasts_through_the_file, sql);

/// The file's connection is its own: queries the session runs meanwhile (the sidebar, another
/// editor tab) can neither take it nor lose the state the file has set up on it.
async fn other_queries_cannot_take_the_files_connection(c: &Ctx) {
    let temp = c.name("held");
    let copy = c.scratch("held_copy");
    let (qtemp, qcopy) = (c.q(&temp), c.qt(&copy));
    let (int, _) = types(c);
    let create_temp = if c.is_my() {
        "CREATE TEMPORARY TABLE"
    } else {
        "CREATE TEMP TABLE"
    };
    let mut dump = format!("{create_temp} {qtemp} (a {int});\n");
    for i in 0..300 {
        dump.push_str(&format!("INSERT INTO {qtemp} VALUES ({i});\n"));
    }
    dump.push_str(&format!("CREATE TABLE {qcopy} AS SELECT a FROM {qtemp};\n"));

    let stop = Arc::new(AtomicBool::new(false));
    let busy = {
        let (driver, database, stop) = (c.driver.clone(), c.database.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                if let Ok(cursor) = driver.query(Some(&database), "SELECT 1").await {
                    let _ = drain(cursor).await;
                }
            }
        })
    };
    let done = import(c, &dump).await;
    stop.store(true, Ordering::Relaxed);
    busy.await.unwrap();

    assert_eq!((done.rows, &done.error), (302, &None));
    assert_eq!(c.side_count(&copy).await, 300);
}
each_engine!(other_queries_cannot_take_the_files_connection, sql);

async fn an_open_transaction_is_rolled_back(c: &Ctx) {
    let t = c.scratch("tx");
    let missing = c.name("missing");
    let (qt, qm) = (c.qt(&t), c.qt(&missing));
    let (int, _) = types(c);
    let begin = if c.is_my() {
        "START TRANSACTION"
    } else {
        "BEGIN"
    };
    c.side_exec(&format!("CREATE TABLE {qt} (a {int})")).await;

    // The file fails inside its transaction: nothing of it stays, and the connection the next
    // statement uses is not left in an aborted transaction (Postgres would refuse everything).
    let dump =
        format!("{begin};\nINSERT INTO {qt} VALUES (1);\nINSERT INTO {qm} VALUES (2);\nCOMMIT;\n");
    let done = import(c, &dump).await;
    let error = done.error.expect("an error");
    assert!(error.starts_with("Statement 3 failed: "), "{error}");
    assert!(
        error.ends_with("The transaction the file had opened was rolled back."),
        "{error}"
    );
    assert_eq!(c.side_count(&t).await, 0);
    assert_eq!(
        c.scalar(&format!("SELECT count(*) FROM {qt}")).await,
        Value::Int(0)
    );

    // A file that forgets its COMMIT is rolled back as well, with a warning.
    let dump = format!("{begin};\nINSERT INTO {qt} VALUES (1);\n");
    let done = import(c, &dump).await;
    assert_eq!(done.error, None);
    assert!(done.warning.expect("a warning").contains("rolled back"));
    assert_eq!(c.side_count(&t).await, 0);
    assert_eq!(
        c.scalar(&format!("SELECT count(*) FROM {qt}")).await,
        Value::Int(0)
    );

    // A complete transaction is committed.
    let dump =
        format!("{begin};\nINSERT INTO {qt} VALUES (1);\nINSERT INTO {qt} VALUES (2);\nCOMMIT;\n");
    let done = import(c, &dump).await;
    assert_eq!((done.rows, done.error, done.warning), (4, None, None));
    assert_eq!(c.side_count(&t).await, 2);
}
each_engine!(an_open_transaction_is_rolled_back, sql);

async fn read_only_connections_only_read(c: &Ctx) {
    let t = c.scratch("ro");
    let qt = c.qt(&t);
    let (int, _) = types(c);
    c.side_exec(&format!("CREATE TABLE {qt} (a {int})")).await;
    c.side_exec(&format!("INSERT INTO {qt} VALUES (1)")).await;

    let done = import_with(c, &format!("SELECT a FROM {qt};\nSELECT 2;\n"), true, true).await;
    assert_eq!((done.rows, &done.error), (2, &None));

    for stop_on_error in [true, false] {
        let dump = format!("SELECT 1;\nDELETE FROM {qt};\nSELECT 3;\n");
        let done = import_with(c, &dump, stop_on_error, true).await;
        assert_eq!(
            done.error.as_deref(),
            Some("Statement 2 failed: This connection is read-only."),
            "stop_on_error {stop_on_error}"
        );
        assert_eq!(done.rows, 1);
        assert_eq!(
            c.side_count(&t).await,
            1,
            "the DELETE never reached the server"
        );
    }
}
each_engine!(read_only_connections_only_read, sql);

// ---------------------------------------------------------------------------
// Size
// ---------------------------------------------------------------------------

/// More than two read pieces (4 MB each), with statements that straddle both boundaries and
/// text that must reach the server exactly.
async fn a_dump_bigger_than_one_read_piece(c: &Ctx) {
    let t = c.scratch("big");
    let qt = c.qt(&t);
    let (int, _) = types(c);
    let text = if c.is_pg() {
        "text"
    } else if c.is_sqlite() {
        "TEXT"
    } else {
        "LONGTEXT"
    };
    let mut dump = format!("CREATE TABLE {qt} (id {int} PRIMARY KEY, body {text});\n");
    // 100 rows of ~90 KB: ~9 MB. The text has semicolons, quotes and multi-byte characters
    // so a cut anywhere is likely to land inside one.
    let unit = "é; it''s -- 日本 ";
    for id in 0..100 {
        let body = unit.repeat(5_000 + id);
        dump.push_str(&format!("INSERT INTO {qt} VALUES ({id}, '{body}');\n"));
    }
    assert!(dump.len() > 9 * 1024 * 1024, "{}", dump.len());
    let done = import(c, &dump).await;
    assert_eq!((done.rows, &done.error, &done.warning), (101, &None, &None));
    assert_eq!(done.bytes, dump.len() as u64);
    assert_eq!(c.side_count(&t).await, 100);
    // Characters, not bytes: 14 per unit ("é; it's -- 日本 " is 14 characters).
    let units = "é; it's -- 日本 ".chars().count() as i64;
    let length = match c.engine {
        Engine::Sqlite | Engine::Postgres => "length",
        _ => "CHAR_LENGTH",
    };
    for id in [0_i64, 41, 99] {
        assert_eq!(
            c.side_text(&format!("SELECT {length}(body) FROM {qt} WHERE id = {id}"))
                .await
                .unwrap()
                .parse::<i64>()
                .unwrap(),
            units * (5_000 + id),
            "row {id}"
        );
    }
}
each_engine!(a_dump_bigger_than_one_read_piece, sql);

// ---------------------------------------------------------------------------
// Cancelling
// ---------------------------------------------------------------------------

/// SQL that runs for a minute unless it is cancelled, tagged so the query log shows when it starts.
fn endless(c: &Ctx, marker: &str) -> String {
    match c.engine {
        Engine::Postgres => format!("SELECT /* {marker} */ pg_sleep(60)"),
        Engine::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) SELECT /* {marker} */ count(*) FROM c"
        ),
        _ => format!("SELECT /* {marker} */ SLEEP(60)"),
    }
}

async fn cancelling_interrupts_the_running_statement(c: &Ctx) {
    let before = c.scratch("cancel_before");
    let after = c.scratch("cancel_after");
    let (qb, qa) = (c.qt(&before), c.qt(&after));
    let (int, _) = types(c);
    let marker = c.name("endless");
    let dump = format!(
        "CREATE TABLE {qb} (a {int});\nINSERT INTO {qb} VALUES (1);\n{};\nCREATE TABLE {qa} (a {int});\n",
        endless(c, &marker)
    );
    let dir = TempDir::new();
    let path = dir.file("slow.sql", dump.as_bytes());
    let cancel = CancellationToken::new();
    // Cancel once the slow statement has been sent (it is in the query log by then).
    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    let canceller = {
        let cancel = cancel.clone();
        let marker = marker.clone();
        tokio::spawn(async move {
            let sent = async {
                while !querylog::since(seq).iter().any(|e| e.sql.contains(&marker)) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(60), sent).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let done = tokio::time::timeout(
        Duration::from_secs(30),
        run_sql_import(
            c.driver.clone(),
            spec(c, path, true),
            false,
            Arc::new(|_| {}),
            cancel,
        ),
    )
    .await
    .expect("the import was still running 30 s after it was cancelled")
    .unwrap();
    canceller.await.unwrap();

    assert!(done.done && done.cancelled, "{done:?}");
    assert_eq!(done.error, None);
    assert_eq!(done.rows, 2, "the statements before the slow one ran");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(c.side_count(&before).await, 1);
    // The statement after the slow one never ran.
    let exists = match c.engine {
        Engine::Postgres => format!(
            "SELECT count(*)::text FROM information_schema.tables WHERE table_name = '{after}'"
        ),
        Engine::Sqlite => format!("SELECT count(*) FROM sqlite_master WHERE name = '{after}'"),
        _ => format!(
            "SELECT CAST(count(*) AS CHAR) FROM information_schema.tables WHERE table_name = '{after}'"
        ),
    };
    assert_eq!(c.side_text(&exists).await.as_deref(), Some("0"));
    // The driver is usable straight away.
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(cancelling_interrupts_the_running_statement, sql);

// ---------------------------------------------------------------------------
// Engine-specific dumps
// ---------------------------------------------------------------------------

/// What mysqldump writes: executable comments, session variables, LOCK TABLES, DELIMITER.
async fn mysql_dump_with_delimiters_and_session_settings(c: &Ctx) {
    let (t, child, parent, proc) = (
        c.scratch("my_t"),
        c.scratch("my_child"),
        c.scratch("my_parent"),
        c.name("my_proc"),
    );
    c.cleanup_sql(format!("DROP PROCEDURE IF EXISTS {}", c.q(&proc)));
    let (qt, qchild, qparent, qproc) = (c.qt(&t), c.qt(&child), c.qt(&parent), c.q(&proc));
    let dump = format!(
        "-- MySQL dump 10.13  Distrib 8.4.0, for Linux (x86_64)\n\
         /*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;\n\
         /*!40101 SET NAMES utf8mb4 */;\n\
         /*!40014 SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS=0 */;\n\
         /*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;\n\
         DROP TABLE IF EXISTS {qt};\n\
         /*!40101 SET @saved_cs_client     = @@character_set_client */;\n\
         CREATE TABLE {qt} (\n  `id` int NOT NULL,\n  `s` varchar(50) DEFAULT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;\n\
         /*!40101 SET character_set_client = @saved_cs_client */;\n\
         LOCK TABLES {qt} WRITE;\n\
         /*!40000 ALTER TABLE {qt} DISABLE KEYS */;\n\
         INSERT INTO {qt} VALUES (1,'a;b'),(2,'it\\'s'),(3,'back\\\\slash');\n\
         /*!40000 ALTER TABLE {qt} ENABLE KEYS */;\n\
         UNLOCK TABLES;\n\
         SET @n = 10;\n\
         INSERT INTO {qt} VALUES (@n, 'from a variable');\n\
         CREATE TABLE {qchild} (id int PRIMARY KEY, p int, FOREIGN KEY (p) REFERENCES {qparent} (id));\n\
         CREATE TABLE {qparent} (id int PRIMARY KEY);\n\
         DELIMITER ;;\n\
         CREATE PROCEDURE {qproc}()\nBEGIN\n  INSERT INTO {qt} VALUES (20, 'p1');\n  INSERT INTO {qt} VALUES (21, 'p2');\nEND ;;\n\
         DELIMITER ;\n\
         CALL {qproc}();\n\
         /*!40014 SET FOREIGN_KEY_CHECKS=@OLD_FOREIGN_KEY_CHECKS */;\n\
         /*!40111 SET SQL_NOTES=@OLD_SQL_NOTES */;\n\
         -- Dump completed\n"
    );
    let done = import(c, &dump).await;
    assert_eq!(done.error, None, "{done:?}");
    assert_eq!(done.warning, None);
    assert_eq!(done.bytes, dump.len() as u64);
    assert_eq!(
        c.side_column(&format!("SELECT s FROM {qt} ORDER BY id"))
            .await,
        [
            Some("a;b".to_owned()),
            Some("it's".to_owned()),
            Some("back\\slash".to_owned()),
            Some("from a variable".to_owned()),
            Some("p1".to_owned()),
            Some("p2".to_owned()),
        ]
    );
}
each_engine!(
    mysql_dump_with_delimiters_and_session_settings,
    mysql,
    mariadb
);

/// What `sqlite3 .dump` writes.
async fn sqlite_dump_with_a_trigger(c: &Ctx) {
    let (t, log) = (c.scratch("lite_t"), c.scratch("lite_log"));
    let (qt, qlog) = (c.qt(&t), c.qt(&log));
    let trigger = c.q(&c.name("lite_trg"));
    let dump = format!(
        "PRAGMA foreign_keys=OFF;\n\
         BEGIN TRANSACTION;\n\
         CREATE TABLE {qt} (id INTEGER PRIMARY KEY AUTOINCREMENT, s TEXT);\n\
         INSERT INTO {qt} VALUES(1,'one');\n\
         INSERT INTO {qt} VALUES(2,'two''s');\n\
         DELETE FROM sqlite_sequence;\n\
         INSERT INTO sqlite_sequence VALUES('{t}',2);\n\
         CREATE TABLE {qlog} (note TEXT);\n\
         CREATE TRIGGER {trigger} AFTER INSERT ON {qt}\n\
         BEGIN\n  INSERT INTO {qlog} VALUES ('inserted ' || new.s);\n  UPDATE {qt} SET s = s || '!' WHERE id = new.id;\nEND;\n\
         INSERT INTO {qt}(s) VALUES ('three');\n\
         COMMIT;\n"
    );
    let done = import(c, &dump).await;
    assert_eq!((&done.error, &done.warning), (&None, &None), "{done:?}");
    assert_eq!(done.rows, 11);
    assert_eq!(
        c.side_column(&format!("SELECT s FROM {qt} ORDER BY id"))
            .await,
        [
            Some("one".to_owned()),
            Some("two's".to_owned()),
            Some("three!".to_owned())
        ]
    );
    assert_eq!(c.side_count(&log).await, 1);
}
each_engine!(sqlite_dump_with_a_trigger, sqlite);

/// What pg_dump writes in plain format, with COPY data blocks.
async fn postgres_dump_with_copy_blocks(c: &Ctx) {
    let (t, u) = (c.scratch("pg_t"), c.scratch("pg_u"));
    let func = c.name("pg_fn");
    c.cleanup_sql(format!("DROP FUNCTION IF EXISTS {}()", c.qt(&func)));
    let (qt, qu, qfunc) = (c.qt(&t), c.qt(&u), c.qt(&func));
    let dump = format!(
        "--\n-- PostgreSQL database dump\n--\n\n\
         \\restrict a1B2c3\n\n\
         SET statement_timeout = 0;\n\
         SET client_encoding = 'UTF8';\n\
         SELECT pg_catalog.set_config('search_path', '', false);\n\n\
         CREATE TABLE {qt} (id integer NOT NULL, name text);\n\n\
         CREATE FUNCTION {qfunc}() RETURNS integer\n    LANGUAGE plpgsql\n    AS $$\nBEGIN\n  RETURN 41 + 1;\nEND;\n$$;\n\n\
         COPY {qt} (id, name) FROM stdin;\n\
         1\tO'Brien\n2\ta;b\n3\t\"never closed\n\\.\n\n\
         CREATE TABLE {qu} (id integer);\n\n\
         INSERT INTO {qu} VALUES ({qfunc}());\n\n\
         \\unrestrict a1B2c3\n"
    );

    // Stopping at errors: the COPY block is the first thing the import cannot do.
    let done = import(c, &dump).await;
    let error = done.error.expect("COPY ... FROM stdin is refused");
    assert!(
        error.starts_with("Statement 6 failed: COPY ... FROM stdin blocks can't be imported"),
        "{error}"
    );
    assert!(error.contains("pg_dump --inserts"), "{error}");
    assert_eq!(done.rows, 5);
    assert_eq!(
        c.side_count(&t).await,
        0,
        "the data lines were not run as SQL"
    );

    // Carrying on: the block is skipped as a whole, everything after it runs.
    c.side_exec(&format!("DROP TABLE {qt}")).await;
    c.side_exec(&format!("DROP FUNCTION {qfunc}()")).await;
    let done = import_with(c, &dump, false, false).await;
    assert_eq!(done.error, None, "{done:?}");
    let warning = done.warning.expect("a warning");
    assert!(
        warning.starts_with("1 of 8 statements failed; first: statement 6: COPY ... FROM stdin"),
        "{warning}"
    );
    assert_eq!(done.rows, 7);
    assert_eq!(done.bytes, dump.len() as u64);
    assert_eq!(c.side_count(&t).await, 0);
    assert_eq!(
        c.side_text(&format!("SELECT id FROM {qu}"))
            .await
            .as_deref(),
        Some("42")
    );
}
each_engine!(postgres_dump_with_copy_blocks, postgres);

async fn postgres_connect_commands_end_the_import(c: &Ctx) {
    let t = c.scratch("pg_connect");
    let qt = c.qt(&t);
    let dump = format!("SELECT 1;\n\\connect other_db\nCREATE TABLE {qt} (a integer);\n");
    for stop_on_error in [true, false] {
        let done = import_with(c, &dump, stop_on_error, false).await;
        let error = done.error.expect("an error");
        assert!(
            error.starts_with("Statement 2 failed: \\connect switches database"),
            "{error}"
        );
        assert_eq!(done.rows, 1);
    }
    let exists = c
        .side_text(&format!(
            "SELECT count(*)::text FROM information_schema.tables WHERE table_name = '{t}'"
        ))
        .await;
    assert_eq!(exists.as_deref(), Some("0"), "nothing ran after \\connect");
}
each_engine!(postgres_connect_commands_end_the_import, postgres);

// ---------------------------------------------------------------------------
// MongoDB
// ---------------------------------------------------------------------------

async fn mongo_refuses_sql_files(c: &Ctx) {
    let dir = TempDir::new();
    let path = dir.file("dump.sql", b"SELECT 1;");
    let import = SqlImport {
        path,
        database: None,
        stop_on_error: true,
    };
    match check(Engine::Mongodb, &import).await {
        Err(Error::InvalidInput(m)) => assert!(m.contains("MongoDB"), "{m}"),
        other => panic!("{other:?}"),
    }
    let result = run_sql_import(
        c.driver.clone(),
        import,
        false,
        Arc::new(|_| {}),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
}
each_engine!(mongo_refuses_sql_files, mongo);
