//! Cancelling long statements and abandoning cursors: the server must stop working,
//! the app must get its connection back, and nothing may leak.

#[macro_use]
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use dbcore::driver::{CancelHandle, Cursor, CursorEvent, Driver};
use dbcore::{Engine, Error, Value};

/// SQL that runs "forever" on `c`, tagged with a marker comment so the test can find it
/// in the server's process list, plus whether a cancel makes it fail (MySQL's SLEEP
/// instead returns 1 when interrupted).
fn endless(c: &Ctx, marker: &str) -> String {
    match c.engine {
        Engine::Postgres => format!("SELECT /* {marker} */ pg_sleep(60)"),
        Engine::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) SELECT /* {marker} */ count(*) FROM c"
        ),
        _ => format!("SELECT /* {marker} */ SLEEP(60)"),
    }
}

/// A CPU-bound query that errors when killed on every engine.
fn endless_cpu(c: &Ctx, marker: &str) -> String {
    match c.engine {
        Engine::Postgres => format!(
            "SELECT /* {marker} */ count(*) FROM (SELECT generate_series(1::bigint, 100000000000) AS g) s WHERE g % 7 = 3"
        ),
        Engine::Sqlite => endless(c, marker),
        _ => format!(
            "SELECT /* {marker} */ COUNT(*) FROM big_table a CROSS JOIN big_table b WHERE a.id + b.id = -1"
        ),
    }
}

/// How many statements containing `marker` the server is running right now.
async fn running(c: &Ctx, marker: &str) -> i64 {
    match c.engine {
        Engine::Postgres => c
            .side_text(&format!(
                "SELECT count(*)::text FROM pg_stat_activity WHERE state = 'active' AND query LIKE '%{marker}%' AND pid <> pg_backend_pid()"
            ))
            .await,
        Engine::Sqlite => Some("0".to_owned()),
        _ => c
            .side_text(&format!(
                "SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.processlist WHERE command = 'Query' AND info LIKE '%{marker}%' AND id <> CONNECTION_ID()"
            ))
            .await,
    }
    .unwrap()
    .parse()
    .unwrap()
}

async fn wait_until_idle(c: &Ctx, marker: &str) {
    eventually(
        Duration::from_secs(5),
        "the server stops running the statement",
        || async { running(c, marker).await == 0 },
    )
    .await;
}

/// Reads a cursor until it ends; the error that ended it, if any. A statement that keeps
/// running long after it was cancelled fails the test instead of hanging it.
async fn finish(cursor: &mut Box<dyn Cursor>) -> Result<usize, Error> {
    let read = async {
        let mut rows = 0;
        while let Some(event) = cursor.next(100).await? {
            if let CursorEvent::Rows(r) = event {
                rows += r.len();
            }
        }
        Ok(rows)
    };
    tokio::time::timeout(Duration::from_secs(30), read)
        .await
        .expect("the statement was still running 30 s after it was cancelled")
}

/// Cancels after `delay` in the background; await the handle to know the cancel has returned.
fn cancel_after(handle: Arc<dyn CancelHandle>, delay: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        handle.cancel().await.unwrap();
    })
}

// ---------------------------------------------------------------------------
// Cancelling
// ---------------------------------------------------------------------------

async fn cancelling_a_long_query_finishes_quickly(c: &Ctx) {
    let marker = c.name("cancel");
    let mut cursor = c
        .driver
        .query(Some(&c.database), &endless_cpu(c, &marker))
        .await
        .unwrap();
    cancel_after(cursor.cancel_handle(), Duration::from_millis(400));
    let started = Instant::now();
    let outcome = finish(&mut cursor).await;
    assert!(
        matches!(outcome, Err(Error::Cancelled)),
        "a killed statement reports Cancelled, got {outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    drop(cursor);
    wait_until_idle(c, &marker).await;
    // The connection that ran it is usable straight away.
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(cancelling_a_long_query_finishes_quickly, sql);

async fn cancelling_sleep(c: &Ctx) {
    // pg_sleep is cancelled with an error; MySQL's SLEEP returns 1 when interrupted.
    let marker = c.name("sleep");
    let mut cursor = c
        .driver
        .query(Some(&c.database), &endless(c, &marker))
        .await
        .unwrap();
    cancel_after(cursor.cancel_handle(), Duration::from_millis(300));
    let started = Instant::now();
    let mut rows = Vec::new();
    let outcome = loop {
        match cursor.next(10).await {
            Ok(Some(CursorEvent::Rows(r))) => rows.extend(r),
            Ok(Some(_)) => {}
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    match (c.engine, outcome) {
        (Engine::Mysql | Engine::Mariadb, Ok(())) => assert_eq!(rows, vec![vec![Value::Int(1)]]),
        (_, Err(Error::Cancelled)) => {}
        (engine, other) => panic!("{engine:?}: {other:?}"),
    }
    drop(cursor);
    wait_until_idle(c, &marker).await;
    assert_eq!(c.scalar("SELECT 2").await, Value::Int(2));
}
each_engine!(cancelling_sleep, postgres, mysql, mariadb);

async fn cancelling_twice_or_too_late_does_nothing(c: &Ctx) {
    // A handle of a statement that has finished must not hit the next one.
    let mut cursor = c
        .driver
        .query(Some(&c.database), "SELECT 1 AS one")
        .await
        .unwrap();
    let stale = cursor.cancel_handle();
    assert_eq!(finish(&mut cursor).await.unwrap(), 1);
    drop(cursor);
    stale.cancel().await.unwrap();

    let next_sql = match c.engine {
        Engine::Postgres => "SELECT pg_sleep(0.6), 'done' AS state",
        Engine::Sqlite => {
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 2000000) SELECT count(*), 'done' AS state FROM c"
        }
        _ => "SELECT SLEEP(0.6), 'done' AS state",
    };
    let mut next = c.driver.query(Some(&c.database), next_sql).await.unwrap();
    // The stale handle fires while the next statement (probably on the same connection) runs.
    stale.cancel().await.unwrap();
    stale.cancel().await.unwrap();
    let mut got = Vec::new();
    while let Some(event) = next
        .next(10)
        .await
        .expect("the later statement is not cancelled")
    {
        if let CursorEvent::Rows(r) = event {
            got.extend(r);
        }
    }
    assert_eq!(got.len(), 1);
    assert_eq!(got[0][1], Value::Text("done".into()));

    // Cancelling the same statement twice is harmless.
    let marker = c.name("twice");
    let mut cursor = c
        .driver
        .query(Some(&c.database), &endless_cpu(c, &marker))
        .await
        .unwrap();
    let handle = cursor.cancel_handle();
    cancel_after(handle.clone(), Duration::from_millis(200));
    cancel_after(handle, Duration::from_millis(250));
    let outcome = finish(&mut cursor).await;
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    drop(cursor);
    wait_until_idle(c, &marker).await;
    assert_eq!(c.scalar("SELECT 3").await, Value::Int(3));
}
each_engine!(cancelling_twice_or_too_late_does_nothing, sql);

async fn a_cancel_sent_before_the_statement_starts_still_stops_it(c: &Ctx) {
    // The cancel races the statement to the server and can win; it must not be lost.
    let marker = c.name("early_cancel");
    for _ in 0..15 {
        let mut cursor = c
            .driver
            .query(Some(&c.database), &endless_cpu(c, &marker))
            .await
            .unwrap();
        let handle = cursor.cancel_handle();
        let started = Instant::now();
        let canceller = tokio::spawn(async move { handle.cancel().await });
        let outcome = finish(&mut cursor).await;
        assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
        canceller.await.unwrap().unwrap();
    }
    wait_until_idle(c, &marker).await;
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(
    a_cancel_sent_before_the_statement_starts_still_stops_it,
    sql
);

async fn cancelling_keeps_the_open_transaction(c: &Ctx) {
    let t = c.make_items("cancel_tx", 2).await;
    let table = c.qt(&t);
    let begin = if c.is_my() {
        "START TRANSACTION"
    } else {
        "BEGIN"
    };
    c.run(begin).await;
    c.run(&format!(
        "INSERT INTO {table} (id, name, grp) VALUES (10, 'in tx', 1)"
    ))
    .await;
    let marker = c.name("tx");
    let mut cursor = c
        .driver
        .query(Some(&c.database), &endless_cpu(c, &marker))
        .await
        .unwrap();
    cancel_after(cursor.cancel_handle(), Duration::from_millis(300));
    let outcome = finish(&mut cursor).await;
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    drop(cursor);
    wait_until_idle(c, &marker).await;
    if c.is_pg() {
        // The transaction is aborted but still open on the same connection.
        let err = c.events("SELECT 1").await.unwrap_err();
        assert!(
            matches!(&err, Error::Database { code: Some(code), .. } if code == "25P02"),
            "{err:?}"
        );
        c.run("ROLLBACK").await;
        assert_eq!(c.side_count(&t).await, 2);
    } else {
        // MySQL and SQLite keep the transaction; the work done before the cancel is still there.
        assert_eq!(
            c.scalar(&format!("SELECT COUNT(*) FROM {table}")).await,
            Value::Int(3)
        );
        c.run("COMMIT").await;
        assert_eq!(c.side_count(&t).await, 3);
    }
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(cancelling_keeps_the_open_transaction, sql);

async fn cancelling_a_write_rolls_the_statement_back(c: &Ctx) {
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("BIGINT", "VARCHAR(50)"),
    };
    let t = c.scratch("cancel_write");
    let table = c.qt(&t);
    c.side_exec(&format!(
        "CREATE TABLE {table} (id {int} PRIMARY KEY, v {text})"
    ))
    .await;
    c.side_exec(&format!("INSERT INTO {table} VALUES (0, 'seed')"))
        .await;
    let marker = c.name("write");
    let insert = match c.engine {
        Engine::Postgres => format!(
            "INSERT INTO {table} SELECT /* {marker} */ g, 'x' FROM (SELECT generate_series(1, 1000000000) AS g) s"
        ),
        Engine::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) INSERT INTO {table} SELECT /* {marker} */ x, 'x' FROM c"
        ),
        _ => format!(
            "INSERT INTO {table} SELECT /* {marker} */ a.id * 1000000 + b.id, 'x' FROM big_table a CROSS JOIN big_table b"
        ),
    };
    let mut cursor = c.driver.query(Some(&c.database), &insert).await.unwrap();
    cancel_after(cursor.cancel_handle(), Duration::from_millis(500));
    let outcome = finish(&mut cursor).await;
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    drop(cursor);
    wait_until_idle(c, &marker).await;
    // The half-done insert is gone, and the table is writable (no lock is left behind).
    eventually(
        Duration::from_secs(10),
        "the aborted insert is rolled back",
        || async { c.side_count(&t).await == 1 },
    )
    .await;
    c.side_exec(&format!("INSERT INTO {table} VALUES (1, 'after')"))
        .await;
    assert_eq!(c.side_count(&t).await, 2);
}
each_engine!(cancelling_a_write_rolls_the_statement_back, sql);

// ---------------------------------------------------------------------------
// Dropping cursors
// ---------------------------------------------------------------------------

fn bulk(c: &Ctx, marker: &str) -> String {
    match c.engine {
        // (A set-returning function in FROM is materialized completely before the first
        // row, spilling gigabytes to disk; in the select list it streams.)
        Engine::Postgres => format!(
            "SELECT /* {marker} */ g, md5(g::text) AS h FROM (SELECT generate_series(1::bigint, 200000000) AS g) s"
        ),
        Engine::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) SELECT /* {marker} */ x, hex(x) FROM c"
        ),
        _ => format!("SELECT /* {marker} */ a.id, b.id FROM big_table a CROSS JOIN big_table b"),
    }
}

async fn dropping_a_cursor_stops_the_server(c: &Ctx) {
    let marker = c.name("drop");
    let mut cursor = c
        .driver
        .query(Some(&c.database), &bulk(c, &marker))
        .await
        .unwrap();
    // Read the columns and a first batch, as the grid does, then walk away.
    let Some(CursorEvent::Columns(cols)) = cursor.next(100).await.unwrap() else {
        panic!("columns first")
    };
    assert_eq!(cols.len(), 2);
    let Some(CursorEvent::Rows(rows)) = cursor.next(100).await.unwrap() else {
        panic!("rows next")
    };
    assert!(!rows.is_empty());
    assert!(
        running(c, &marker).await >= i64::from(!c.is_sqlite()),
        "the statement is still producing rows"
    );
    drop(cursor);
    wait_until_idle(c, &marker).await;
    // The same driver is immediately usable.
    let started = Instant::now();
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
    assert!(started.elapsed() < Duration::from_secs(2));
}
each_engine!(dropping_a_cursor_stops_the_server, sql);

async fn dropping_a_cursor_before_reading_anything(c: &Ctx) {
    let marker = c.name("early");
    let cursor = c
        .driver
        .query(Some(&c.database), &endless(c, &marker))
        .await
        .unwrap();
    // Give the statement time to reach the server, then drop without a single `next`.
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(cursor);
    wait_until_idle(c, &marker).await;
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));

    // Dropped at once, before the producer task has even started.
    let marker = c.name("instant");
    drop(
        c.driver
            .query(Some(&c.database), &endless(c, &marker))
            .await
            .unwrap(),
    );
    wait_until_idle(c, &marker).await;
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(dropping_a_cursor_before_reading_anything, sql);

async fn abandoned_cursors_do_not_leak_connections(c: &Ctx) {
    // Far more abandoned cursors than the pool has connections: if any leaked, the pool
    // would run dry and the final query would time out.
    let app = c.name("leak");
    let driver = connect_driver(c.engine, c.file.as_deref(), |cfg| {
        cfg.options.insert("application_name".into(), app.clone());
    })
    .await
    .unwrap();
    let marker = c.name("leakq");
    for i in 0..40 {
        let mut cursor = driver
            .query(Some(&c.database), &bulk(c, &marker))
            .await
            .unwrap();
        let _ = cursor.next(10).await.unwrap(); // columns
        if i % 2 == 0 {
            let _ = cursor.next(10).await.unwrap(); // some rows
        }
        drop(cursor);
    }
    wait_until_idle(c, &marker).await;
    let started = Instant::now();
    let events = drain(
        driver
            .query(Some(&c.database), "SELECT 41 AS answer")
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(sets_of(&events)[0].rows, vec![vec![Value::Int(41)]]);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    if c.is_pg() {
        let open: i64 = c
            .side_text(&format!(
                "SELECT count(*)::text FROM pg_stat_activity WHERE application_name = '{app}'"
            ))
            .await
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            open <= 9,
            "{open} connections for one driver after 40 abandoned cursors"
        );
        let active: i64 = c
            .side_text(&format!("SELECT count(*)::text FROM pg_stat_activity WHERE application_name = '{app}' AND state = 'active' AND pid <> pg_backend_pid()"))
            .await
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(active, 0);
    }
    driver.close().await;
    if c.is_pg() {
        eventually(
            Duration::from_secs(5),
            "closing the driver closes every connection",
            || async {
                c.side_text(&format!(
                    "SELECT count(*)::text FROM pg_stat_activity WHERE application_name = '{app}'"
                ))
                .await
                .as_deref()
                    == Some("0")
            },
        )
        .await;
    }
}
each_engine!(abandoned_cursors_do_not_leak_connections, sql);

async fn cancelled_cursors_do_not_leak_connections(c: &Ctx) {
    let marker = c.name("cancelleak");
    for _ in 0..12 {
        let mut cursor = c
            .driver
            .query(Some(&c.database), &endless_cpu(c, &marker))
            .await
            .unwrap();
        let cancel = cancel_after(cursor.cancel_handle(), Duration::from_millis(100));
        let outcome = finish(&mut cursor).await;
        assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
        // (A cancel still on its way could otherwise hit the next statement on this connection.)
        cancel.await.unwrap();
    }
    wait_until_idle(c, &marker).await;
    let started = Instant::now();
    assert_eq!(c.scalar("SELECT 5").await, Value::Int(5));
    assert!(started.elapsed() < Duration::from_secs(2));
}
each_engine!(cancelled_cursors_do_not_leak_connections, sql);

async fn more_parallel_statements_than_connections_all_finish(c: &Ctx) {
    // 24 statements at once on a pool of 5-8 connections: the extra ones wait their turn.
    let slow = match c.engine {
        Engine::Postgres => "SELECT pg_sleep(0.2), 1",
        Engine::Sqlite => {
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 300000) SELECT count(*) FROM c"
        }
        _ => "SELECT SLEEP(0.2), 1",
    };
    let started = Instant::now();
    let mut jobs = Vec::new();
    for _ in 0..24 {
        let driver: Arc<dyn Driver> = c.driver.clone();
        let db = c.database.clone();
        jobs.push(tokio::spawn(async move {
            let cursor = driver.query(Some(&db), slow).await?;
            drain(cursor).await
        }));
    }
    for job in jobs {
        let events = job.await.unwrap().expect("every statement finishes");
        assert_eq!(sets_of(&events)[0].rows.len(), 1);
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(c.scalar("SELECT 1").await, Value::Int(1));
}
each_engine!(more_parallel_statements_than_connections_all_finish, sql);
