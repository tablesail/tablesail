//! Manual transactions against every SQL engine: a transaction lives on one dedicated
//! connection (rows it inserts are invisible to everybody else until COMMIT, ROLLBACK discards
//! them, closing the session rolls it back), it survives errors and abandoned result sets,
//! and a script that controls transactions itself runs atomically on one connection.

#[macro_use]
mod common;

use std::sync::Arc;

use common::*;
use dbcore::CellInput;
use dbcore::driver::{CursorEvent, Driver};
use dbcore::model::{ChangeSet, ColumnInput, RowChange, StatementSpan};
use dbcore::querylog;
use dbcore::split::split_statements;
use dbcore::transaction::{NO_TRANSACTION, ScriptConnection, Transactions};
use dbcore::{Engine, Error};

const SESSION: &str = "session-1";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Runs one statement on `driver` in the test's database and drains it.
async fn exec(driver: &Arc<dyn Driver>, c: &Ctx, sql: &str) -> dbcore::Result<Vec<CursorEvent>> {
    drain(driver.query(Some(&c.database), sql).await?).await
}

/// Rows of table `t` as seen by `driver`.
async fn count(driver: &Arc<dyn Driver>, c: &Ctx, t: &str) -> i64 {
    let events = exec(driver, c, &format!("SELECT COUNT(*) FROM {}", c.qt(t)))
        .await
        .unwrap();
    let rows = sets_of(&events).remove(0).rows;
    rows[0][0].to_display().parse().unwrap()
}

/// Whether `driver` sees a row with this id.
async fn has(driver: &Arc<dyn Driver>, c: &Ctx, t: &str, id: i64) -> bool {
    let events = exec(
        driver,
        c,
        &format!("SELECT id FROM {} WHERE id = {id}", c.qt(t)),
    )
    .await
    .unwrap();
    !sets_of(&events).remove(0).rows.is_empty()
}

fn insert(c: &Ctx, t: &str, id: i64) -> String {
    format!(
        "INSERT INTO {} (id, name, grp, label) VALUES ({id}, 'tx {id}', 1, NULL)",
        c.qt(t)
    )
}

/// Begins a transaction on the test's driver.
async fn begin(txs: &Transactions, c: &Ctx) -> String {
    txs.begin(SESSION, &c.driver, Some(&c.database))
        .await
        .unwrap()
}

fn driver_of(txs: &Transactions, id: &str) -> Arc<dyn Driver> {
    txs.get(id, Some(SESSION)).unwrap().driver().clone()
}

fn assert_closed(txs: &Transactions, id: &str) {
    match txs.get(id, None) {
        Err(Error::InvalidInput(m)) => assert_eq!(m, NO_TRANSACTION),
        other => panic!(
            "the transaction should be gone: {:?}",
            other.map(|t| t.id.clone())
        ),
    }
}

/// Runs a script statement by statement the way `query_run` does: on its own connection when
/// it controls transactions, otherwise on the pool. Returns whether it got its own connection.
async fn run_script(c: &Ctx, sql: &str) -> dbcore::Result<bool> {
    let spans: Vec<StatementSpan> = split_statements(sql, c.dialect);
    let own = ScriptConnection::open(&c.driver, Some(&c.database), &spans).await?;
    let driver = own
        .as_ref()
        .map_or_else(|| c.driver.clone(), |s| s.driver().clone());
    let mut outcome = Ok(());
    for span in &spans {
        if let Err(e) = exec(&driver, c, &span.sql).await {
            outcome = Err(e);
            break;
        }
    }
    if let Some(own) = &own {
        own.finish().await;
    }
    outcome.map(|()| own.is_some())
}

// ---------------------------------------------------------------------------
// The transaction
// ---------------------------------------------------------------------------

async fn commit_persists_and_uncommitted_rows_are_invisible(c: &Ctx) {
    let t = c.make_items("tx_commit", 3).await;
    let txs = Transactions::default();
    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();
    // The transaction sees its own row; the pool, another driver and a plain side connection do not.
    assert_eq!(count(&tx, c, &t).await, 4);
    assert!(has(&tx, c, &t, 100).await);
    assert_eq!(count(&c.driver, c, &t).await, 3);
    assert!(!has(&c.driver, c, &t, 100).await);
    assert_eq!(c.side_count(&t).await, 3);

    txs.commit(&id).await.unwrap();
    assert_eq!(c.side_count(&t).await, 4);
    assert_eq!(count(&c.driver, c, &t).await, 4);
    assert_closed(&txs, &id);
    // The id cannot be committed or rolled back twice.
    assert!(matches!(txs.commit(&id).await, Err(Error::InvalidInput(_))));
    assert!(matches!(
        txs.rollback(&id).await,
        Err(Error::InvalidInput(_))
    ));

    // BEGIN, the statement and COMMIT went through the query log.
    let log: Vec<String> = querylog::since(seq).into_iter().map(|e| e.sql).collect();
    let begin_sql = if c.is_my() {
        "START TRANSACTION"
    } else {
        "BEGIN"
    };
    assert!(log.iter().any(|s| s == begin_sql), "{begin_sql} not logged");
    assert!(
        log.contains(&insert(c, &t, 100)),
        "the insert is not in the log"
    );
    assert!(log.iter().any(|s| s == "COMMIT"), "COMMIT not logged");
}
each_engine!(commit_persists_and_uncommitted_rows_are_invisible, sql);

async fn rollback_discards(c: &Ctx) {
    let t = c.make_items("tx_rollback", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();
    exec(&tx, c, &format!("DELETE FROM {} WHERE id = 1", c.qt(&t)))
        .await
        .unwrap();
    assert_eq!(count(&tx, c, &t).await, 3);
    assert_eq!(c.side_count(&t).await, 3);

    txs.rollback(&id).await.unwrap();
    assert_eq!(c.side_count(&t).await, 3);
    assert!(!has(&c.driver, c, &t, 100).await);
    assert!(has(&c.driver, c, &t, 1).await);
    assert_closed(&txs, &id);
    // The dedicated connection is closed with it.
    assert!(exec(&tx, c, "SELECT 1").await.is_err());
    // The session's own driver is untouched.
    assert_eq!(count(&c.driver, c, &t).await, 3);
}
each_engine!(rollback_discards, sql);

async fn closing_the_session_rolls_back(c: &Ctx) {
    let t = c.make_items("tx_session", 3).await;
    let txs = Transactions::default();
    let a = begin(&txs, c).await;
    let b = begin(&txs, c).await;
    let other = txs
        .begin("session-2", &c.driver, Some(&c.database))
        .await
        .unwrap();
    // (SQLite lets one transaction write at a time, so only the first one does.)
    exec(&driver_of(&txs, &a), c, &insert(c, &t, 100))
        .await
        .unwrap();
    if !c.is_sqlite() {
        exec(&driver_of(&txs, &b), c, &insert(c, &t, 101))
            .await
            .unwrap();
    }
    let kept = txs.get(&other, Some("session-2")).unwrap().driver().clone();

    // Transactions are looked up per session.
    assert!(matches!(
        txs.get(&a, Some("session-2")),
        Err(Error::InvalidInput(_))
    ));

    txs.rollback_session(SESSION).await;
    assert_closed(&txs, &a);
    assert_closed(&txs, &b);
    assert_eq!(c.side_count(&t).await, 3);
    // Another session's transaction is still open (and, now that the locks of the first ones
    // are released, can write).
    exec(&kept, c, &insert(c, &t, 102)).await.unwrap();
    assert!(has(&kept, c, &t, 102).await);
    assert_eq!(c.side_count(&t).await, 3);

    // Quitting the app rolls back whatever is left.
    txs.rollback_all().await;
    assert_closed(&txs, &other);
    assert_eq!(c.side_count(&t).await, 3);
}
each_engine!(closing_the_session_rolls_back, sql);

async fn dropping_the_pinned_driver_without_rollback_rolls_back(c: &Ctx) {
    // Whatever ends the dedicated connection (a crash, the process exiting), the server rolls back.
    let t = c.make_items("tx_close", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);
    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();
    tx.close().await;
    assert_eq!(c.side_count(&t).await, 3);
    txs.rollback(&id).await.unwrap();
    assert_eq!(c.side_count(&t).await, 3);
}
each_engine!(dropping_the_pinned_driver_without_rollback_rolls_back, sql);

async fn a_failed_statement_leaves_the_transaction_open(c: &Ctx) {
    let t = c.make_items("tx_error", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();
    // A duplicate key.
    let err = exec(&tx, c, &insert(c, &t, 1)).await.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");

    if c.is_pg() {
        // Postgres ends the transaction at the first error: everything is refused until ROLLBACK,
        // and COMMIT would silently roll back, so it is refused too.
        let err = exec(&tx, c, &insert(c, &t, 101)).await.unwrap_err();
        assert!(err.to_string().contains("aborted"), "{err}");
        let err = txs.commit(&id).await.unwrap_err();
        // Not InvalidInput: that kind means "no such transaction" to the caller.
        assert!(
            matches!(&err, Error::Database { code, .. } if code.as_deref() == Some("25P02")),
            "{err:?}"
        );
        assert!(err.to_string().contains("rolled back"), "{err}");
        // The refused commit leaves the transaction in place for the rollback.
        assert!(txs.get(&id, Some(SESSION)).is_ok());
        txs.rollback(&id).await.unwrap();
        assert_eq!(c.side_count(&t).await, 3);
    } else {
        // The other engines undo the statement only; the transaction goes on.
        exec(&tx, c, &insert(c, &t, 101)).await.unwrap();
        assert_eq!(count(&tx, c, &t).await, 5);
        txs.commit(&id).await.unwrap();
        assert_eq!(c.side_count(&t).await, 5);
    }
    assert_closed(&txs, &id);
}
each_engine!(a_failed_statement_leaves_the_transaction_open, sql);

/// A COMMIT the server refuses (a deferred foreign key): SQLite leaves the transaction open for
/// a fix and a retry, but Postgres has already rolled it back, so nothing of it may stay
/// registered - later statements would silently run outside any transaction.
async fn a_refused_commit_is_handled_as_the_engine_does(c: &Ctx) {
    let parent = c.scratch("tx_parent");
    let child = c.scratch("tx_child"); // dropped first
    let (qp, qc) = (c.qt(&parent), c.qt(&child));
    c.side_exec(&format!("CREATE TABLE {qp} (id INTEGER PRIMARY KEY)"))
        .await;
    c.side_exec(&format!(
        "CREATE TABLE {qc} (id INTEGER PRIMARY KEY, pid INTEGER REFERENCES {qp} (id) \
         DEFERRABLE INITIALLY DEFERRED)"
    ))
    .await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);
    exec(
        &tx,
        c,
        &format!("INSERT INTO {qc} (id, pid) VALUES (1, 99)"),
    )
    .await
    .unwrap();

    let err = txs.commit(&id).await.unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    assert_eq!(c.side_count(&child).await, 0, "nothing was committed");

    if c.is_pg() {
        // The server rolled the transaction back; the error says so, and the id is gone.
        assert!(err.to_string().contains("rolled back"), "{err}");
        assert_closed(&txs, &id);
        // The connection went with it: nothing can autocommit through the old id.
        assert!(
            exec(
                &tx,
                c,
                &format!("INSERT INTO {qc} (id, pid) VALUES (2, NULL)")
            )
            .await
            .is_err()
        );
        assert_eq!(c.side_count(&child).await, 0);
    } else {
        // The transaction is still open: supply the parent row and commit again.
        assert!(txs.get(&id, Some(SESSION)).is_ok());
        exec(&tx, c, &format!("INSERT INTO {qp} (id) VALUES (99)"))
            .await
            .unwrap();
        txs.commit(&id).await.unwrap();
        assert_eq!(c.side_count(&child).await, 1);
        assert_closed(&txs, &id);
    }
}
each_engine!(
    a_refused_commit_is_handled_as_the_engine_does,
    sqlite,
    postgres
);

async fn savepoints_recover_from_errors(c: &Ctx) {
    let t = c.make_items("tx_savepoint", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();
    exec(&tx, c, "SAVEPOINT before_mistake").await.unwrap();
    exec(&tx, c, &insert(c, &t, 1)).await.unwrap_err();
    exec(&tx, c, "ROLLBACK TO SAVEPOINT before_mistake")
        .await
        .unwrap();
    exec(&tx, c, &insert(c, &t, 101)).await.unwrap();
    // The transaction is healthy again, so COMMIT is accepted.
    txs.commit(&id).await.unwrap();
    assert!(has(&c.driver, c, &t, 100).await);
    assert!(has(&c.driver, c, &t, 101).await);
    assert_eq!(c.side_count(&t).await, 5);
}
each_engine!(savepoints_recover_from_errors, sql);

async fn dropping_a_result_set_keeps_the_transaction(c: &Ctx) {
    // A result bigger than the cursor's buffer, abandoned after its first rows (the grid only
    // reads what the user scrolls to). A pooled connection would be thrown away; the one that
    // holds a transaction must finish reading the statement instead, and the next statement
    // waits for it.
    let t = c.make_items("tx_drop", 6000).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    exec(&tx, c, &insert(c, &t, 10_000)).await.unwrap();
    for _ in 0..3 {
        let mut cursor = tx
            .query(Some(&c.database), &format!("SELECT * FROM {}", c.qt(&t)))
            .await
            .unwrap();
        let mut seen = 0;
        while seen == 0 {
            if let Some(CursorEvent::Rows(rows)) = cursor.next(10).await.unwrap() {
                seen = rows.len();
            }
        }
        drop(cursor);
    }
    // Still the same transaction: its own insert is there, and more can be added.
    assert!(has(&tx, c, &t, 10_000).await);
    exec(&tx, c, &insert(c, &t, 10_001)).await.unwrap();
    assert_eq!(c.side_count(&t).await, 6000);
    txs.commit(&id).await.unwrap();
    assert_eq!(c.side_count(&t).await, 6002);
}
each_engine!(dropping_a_result_set_keeps_the_transaction, sql);

async fn statements_take_turns_on_the_connection(c: &Ctx) {
    let t = c.make_items("tx_turns", 6000).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);

    // Two cursors opened at once: the second waits until the first has been read to its end
    // (it has more rows than the cursor buffers, so it is held up until the consumer reads).
    let first = tx
        .query(Some(&c.database), &format!("SELECT * FROM {}", c.qt(&t)))
        .await
        .unwrap();
    let second = {
        let tx = tx.clone();
        let sql = insert(c, &t, 100_000);
        let database = c.database.clone();
        tokio::spawn(async move { drain(tx.query(Some(&database), &sql).await?).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "the second statement overtook the first"
    );
    let rows = drain(first).await.unwrap();
    assert_eq!(sets_of(&rows)[0].rows.len(), 6000);
    second.await.unwrap().unwrap();
    assert_eq!(count(&tx, c, &t).await, 6001);
    txs.rollback(&id).await.unwrap();
}
each_engine!(statements_take_turns_on_the_connection, sql);

async fn cancelling_a_statement_keeps_the_connection(c: &Ctx) {
    // Cancel is a statement-level event: the connection (and so the transaction) stays, even if
    // the server then refuses everything until ROLLBACK (Postgres).
    let t = c.make_items("tx_cancel", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);
    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();

    let endless = match c.engine {
        Engine::Postgres => "SELECT pg_sleep(60)".to_owned(),
        Engine::Sqlite => "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000000) SELECT count(*) FROM c".to_owned(),
        _ => "SELECT SLEEP(60)".to_owned(),
    };
    let mut cursor = tx.query(Some(&c.database), &endless).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    cursor.cancel_handle().cancel().await.unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while cursor.next(100).await?.is_some() {}
        Ok::<(), Error>(())
    })
    .await
    .expect("the cancelled statement should end");
    // (MySQL's SLEEP just returns early instead of failing.)
    assert!(
        ended.is_ok() || matches!(ended, Err(Error::Cancelled)),
        "{ended:?}"
    );
    drop(cursor);

    if c.is_pg() {
        assert!(
            exec(&tx, c, "SELECT 1")
                .await
                .unwrap_err()
                .to_string()
                .contains("aborted")
        );
        assert!(txs.commit(&id).await.is_err());
        txs.rollback(&id).await.unwrap();
        assert_eq!(c.side_count(&t).await, 3);
    } else {
        assert!(
            has(&tx, c, &t, 100).await,
            "the transaction survived the cancel"
        );
        exec(&tx, c, &insert(c, &t, 101)).await.unwrap();
        txs.commit(&id).await.unwrap();
        assert_eq!(c.side_count(&t).await, 5);
    }
}
each_engine!(cancelling_a_statement_keeps_the_connection, sql);

async fn a_lost_connection_is_not_replaced(c: &Ctx) {
    let t = c.make_items("tx_lost", 3).await;
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);
    exec(&tx, c, &insert(c, &t, 100)).await.unwrap();

    // Kill the backend from outside.
    let (id_sql, kill) = if c.is_pg() {
        ("SELECT pg_backend_pid()", "SELECT pg_terminate_backend")
    } else {
        ("SELECT CONNECTION_ID()", "KILL")
    };
    let pid = exec(&tx, c, id_sql).await.unwrap();
    let pid = sets_of(&pid)[0].rows[0][0].to_display();
    if c.is_pg() {
        c.side_exec(&format!("{kill}({pid})")).await;
    } else {
        c.side_exec(&format!("{kill} {pid}")).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // The statements fail, and none of them runs on a fresh connection outside the transaction.
    assert!(exec(&tx, c, "SELECT 1").await.is_err());
    assert!(exec(&tx, c, &insert(c, &t, 101)).await.is_err());
    assert!(txs.commit(&id).await.is_err());
    assert_eq!(c.side_count(&t).await, 3);
    // Rolling back still cleans up.
    txs.rollback(&id).await.unwrap();
    assert_closed(&txs, &id);
    assert_eq!(c.side_count(&t).await, 3);
}
each_engine!(a_lost_connection_is_not_replaced, postgres, mysql, mariadb);

async fn transactions_are_bound_to_their_database(c: &Ctx) {
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    let tx = driver_of(&txs, &id);
    // The transaction's connection is in one database; asking for another one is refused
    // instead of quietly running somewhere else.
    let err = tx
        .query(Some("some_other_database"), "SELECT 1")
        .await
        .map(drop)
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
    assert!(exec(&tx, c, "SELECT 1").await.is_ok());
    txs.rollback(&id).await.unwrap();
}
each_engine!(transactions_are_bound_to_their_database, postgres);

// ---------------------------------------------------------------------------
// Scripts that control transactions themselves
// ---------------------------------------------------------------------------

async fn scripts_with_begin_and_rollback_leave_no_trace(c: &Ctx) {
    let t = c.make_items("tx_script", 3).await;
    let ins = |id| insert(c, &t, id);

    // BEGIN … ROLLBACK discards.
    let own = run_script(c, &format!("BEGIN; {}; {}; ROLLBACK;", ins(100), ins(101)))
        .await
        .unwrap();
    assert!(own);
    assert_eq!(c.side_count(&t).await, 3);

    // BEGIN … COMMIT persists.
    run_script(c, &format!("BEGIN; {}; COMMIT;", ins(102)))
        .await
        .unwrap();
    assert_eq!(c.side_count(&t).await, 4);
    assert!(has(&c.driver, c, &t, 102).await);

    // A script that ends inside its transaction is rolled back when it is over.
    run_script(c, &format!("BEGIN; {};", ins(103)))
        .await
        .unwrap();
    assert_eq!(c.side_count(&t).await, 4);

    // A statement that fails ends the script; what came before it in the transaction is undone.
    let err = run_script(c, &format!("BEGIN; {}; {}; COMMIT;", ins(104), ins(102)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    assert_eq!(c.side_count(&t).await, 4);

    // The pool was never left in a transaction by any of this.
    c.side_exec(&ins(105)).await;
    assert_eq!(count(&c.driver, c, &t).await, 5);
}
each_engine!(scripts_with_begin_and_rollback_leave_no_trace, sql);

async fn savepoints_in_a_script(c: &Ctx) {
    let t = c.make_items("tx_script_sp", 3).await;
    let sql = format!(
        "BEGIN; {}; SAVEPOINT s; {}; ROLLBACK TO s; COMMIT;",
        insert(c, &t, 100),
        insert(c, &t, 101)
    );
    assert!(run_script(c, &sql).await.unwrap());
    assert!(has(&c.driver, c, &t, 100).await);
    assert!(!has(&c.driver, c, &t, 101).await);
    assert_eq!(c.side_count(&t).await, 4);
}
each_engine!(savepoints_in_a_script, sql);

async fn schema_changes_reach_the_session_drivers_caches(c: &Ctx) {
    // The session driver caches table structures (previewing an edit reads them); the pinned
    // connection's DDL never passes through it, so the end of the transaction has to make it
    // forget them.
    let t = c.make_items("tx_ddl", 3).await;
    let knows = async |column: &str| {
        let set = ChangeSet {
            table: c.tref(&t),
            changes: vec![RowChange::Insert {
                values: vec![
                    ColumnInput {
                        column: "id".into(),
                        value: CellInput::Text("999".into()),
                    },
                    ColumnInput {
                        column: column.into(),
                        value: CellInput::Text("1".into()),
                    },
                ],
            }],
        };
        c.driver.preview_changes(&set).await.is_ok()
    };
    assert!(!knows("extra").await);

    let add = |column: &str| format!("ALTER TABLE {} ADD COLUMN {column} INTEGER", c.qt(&t));
    run_script(c, &format!("BEGIN; {}; COMMIT;", add("extra")))
        .await
        .unwrap();
    assert!(knows("extra").await, "the script's new column is unknown");

    assert!(!knows("extra2").await);
    let txs = Transactions::default();
    let id = begin(&txs, c).await;
    exec(&driver_of(&txs, &id), c, &add("extra2"))
        .await
        .unwrap();
    txs.commit(&id).await.unwrap();
    assert!(
        knows("extra2").await,
        "the transaction's new column is unknown"
    );
}
each_engine!(schema_changes_reach_the_session_drivers_caches, sql);

async fn plain_scripts_stay_on_the_pool(c: &Ctx) {
    let t = c.make_items("tx_plain", 3).await;
    let own = run_script(c, &format!("{}; SELECT 1;", insert(c, &t, 100)))
        .await
        .unwrap();
    assert!(!own);
    assert_eq!(c.side_count(&t).await, 4);
}
each_engine!(plain_scripts_stay_on_the_pool, sql);

// ---------------------------------------------------------------------------
// MongoDB
// ---------------------------------------------------------------------------

async fn mongo_has_no_manual_transactions(c: &Ctx) {
    let txs = Transactions::default();
    let err = txs
        .begin(SESSION, &c.driver, Some(&c.database))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::InvalidInput(m) if m == "Manual transactions aren't supported for MongoDB."),
        "{err:?}"
    );
    assert!(txs.ids_of(SESSION).is_empty());
    assert_eq!(c.driver.engine(), Engine::Mongodb);
}
each_engine!(mongo_has_no_manual_transactions, mongo);
