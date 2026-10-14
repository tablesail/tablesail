//! The query log: every statement TableSail sends shows up, in order, in the console pane.
//!
//! The log is process-wide and these tests run in parallel, so each one finds its own
//! entries by a marker (its scratch table name) and never clears the log.

#[macro_use]
mod common;

use common::*;
use dbcore::model::{ChangeSet, ColumnInput, ColumnValue, FilterOp, RowChange};
use dbcore::querylog::{self, LogEntry};
use dbcore::value::CellInput;
use dbcore::{Engine, Value};

fn newest_seq() -> u64 {
    querylog::since(0).last().map_or(0, |e| e.seq)
}

/// Entries after `seq` that mention `marker`.
fn mentioning(seq: u64, marker: &str) -> Vec<LogEntry> {
    querylog::since(seq)
        .into_iter()
        .filter(|e| e.sql.contains(marker))
        .collect()
}

async fn browsing_records_the_statements(c: &Ctx) {
    let t = c.make_items("log_browse", 30).await;
    let seq = newest_seq();
    let mut req = c.browse(&t);
    req.filters = vec![filter("name", FilterOp::Contains, Some("00007"))];
    req.sort = vec![sort("id", true)];
    let page = c.page(&req, 0, 25).await;
    assert_eq!(page.rows.len(), 1);
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!(count.exact, Some(1));
    let structure = c.driver.table_structure(&c.tref(&t)).await.unwrap();
    assert!(!structure.columns.is_empty());
    let _ = c.all_rows(&req).await;

    let entries = mentioning(seq, &t);
    assert!(
        entries.iter().any(|e| e.sql.starts_with("SELECT")
            && e.sql.contains("00007")
            && e.sql.to_uppercase().contains("LIMIT 25")),
        "the page query is logged with its filter and limit: {entries:#?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e.sql.to_uppercase().contains("COUNT(*)") && e.sql.contains("00007")),
        "the count query is logged: {entries:#?}"
    );
    assert!(
        entries.iter().filter(|e| e.sql.contains("00007")).count() >= 3,
        "page, count and export each log their statement: {entries:#?}"
    );
    // Describing the table runs catalog queries that name it.
    let catalog = querylog::since(seq);
    assert!(
        catalog.len() > entries.len() || c.is_sqlite(),
        "catalog queries are logged too"
    );
    // The page reports the SQL it ran, and that is what the log has.
    assert!(
        entries.iter().any(|e| e.sql == page.query),
        "{} not in {entries:#?}",
        page.query
    );
    // Strictly increasing sequence numbers, real timestamps.
    let all = querylog::since(seq);
    assert!(all.windows(2).all(|w| w[0].seq < w[1].seq));
    assert!(all.iter().all(|e| e.at_ms > 1_600_000_000_000));
    // A poller that has seen everything gets nothing; one that saw nothing gets the lot kept.
    let last = all.last().unwrap().seq;
    assert!(querylog::since(last).iter().all(|e| e.seq > last));
    assert!(querylog::since(u64::MAX).is_empty());
}
each_engine!(browsing_records_the_statements, sql);

async fn applying_changes_records_the_transaction(c: &Ctx) {
    let t = c.make_items("log_apply", 5).await;
    let seq = newest_seq();
    let set = ChangeSet {
        table: c.tref(&t),
        changes: vec![
            RowChange::Insert {
                values: vec![
                    ColumnInput {
                        column: "id".into(),
                        value: CellInput::Text("50".into()),
                    },
                    ColumnInput {
                        column: "name".into(),
                        value: CellInput::Text("logged insert".into()),
                    },
                    ColumnInput {
                        column: "grp".into(),
                        value: CellInput::Text("1".into()),
                    },
                ],
            },
            RowChange::Update {
                key: vec![ColumnValue {
                    column: "id".into(),
                    value: Value::Int(1),
                }],
                original: vec![],
                set: vec![ColumnInput {
                    column: "name".into(),
                    value: CellInput::Text("logged update".into()),
                }],
            },
            RowChange::Delete {
                key: vec![ColumnValue {
                    column: "id".into(),
                    value: Value::Int(2),
                }],
            },
        ],
    };
    c.driver.preview_changes(&set).await.unwrap();
    let applied = c.driver.apply_changes(&set).await.unwrap();
    assert_eq!(applied.rows_affected, 3);

    let mine = mentioning(seq, &t);
    let pos = |needle: &str| {
        mine.iter()
            .position(|e| e.sql.contains(needle))
            .unwrap_or_else(|| panic!("`{needle}` not logged: {mine:#?}"))
    };
    let (insert, update, delete) = (
        pos("logged insert"),
        pos("logged update"),
        pos("DELETE FROM"),
    );
    assert!(
        insert < update && update < delete,
        "statements are logged in the order they ran: {mine:#?}"
    );
    assert!(mine[insert].sql.starts_with("INSERT INTO"));
    assert!(mine[update].sql.starts_with("UPDATE"));
    // BEGIN before the first of them and COMMIT after the last (other tests' may interleave).
    let all = querylog::since(seq);
    let seq_of = |e: &LogEntry| e.seq;
    assert!(
        all.iter()
            .any(|e| e.sql == "BEGIN" && e.seq < seq_of(&mine[insert])),
        "{all:#?}"
    );
    assert!(
        all.iter()
            .any(|e| e.sql == "COMMIT" && e.seq > seq_of(&mine[delete])),
        "{all:#?}"
    );
    // Previewing logged nothing of its own: exactly one INSERT for this table.
    assert_eq!(
        mine.iter()
            .filter(|e| e.sql.starts_with("INSERT INTO"))
            .count(),
        1
    );

    // A failing set logs the statements it tried, and no COMMIT after the failure.
    let seq = newest_seq();
    let bad = ChangeSet {
        table: c.tref(&t),
        changes: vec![RowChange::Insert {
            values: vec![
                ColumnInput {
                    column: "id".into(),
                    value: CellInput::Text("1".into()),
                },
                ColumnInput {
                    column: "name".into(),
                    value: CellInput::Text("logged failure".into()),
                },
                ColumnInput {
                    column: "grp".into(),
                    value: CellInput::Text("1".into()),
                },
            ],
        }],
    };
    assert!(c.driver.apply_changes(&bad).await.is_err());
    let mine = mentioning(seq, "logged failure");
    assert_eq!(mine.len(), 1, "{mine:#?}");
}
each_engine!(applying_changes_records_the_transaction, sql);

async fn queries_are_logged_as_sent(c: &Ctx) {
    let marker = c.name("verbatim");
    let sql = format!(
        "/* {marker} */ SELECT 'héllo; 日本語 🎉' AS \"a b\",\n       1 AS one -- trailing"
    );
    let sql = if c.is_my() {
        sql.replace("\"a b\"", "`a b`")
    } else {
        sql
    };
    let seq = newest_seq();
    let _ = c.run(&sql).await;
    let found = mentioning(seq, &marker);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].sql, sql,
        "logged exactly as typed, comments and all"
    );

    // A statement the server rejects is logged too: it was sent.
    let bad = format!("SELEC /* {marker}_bad */ 1");
    let seq = newest_seq();
    assert!(c.events(&bad).await.is_err());
    let found = mentioning(seq, &format!("{marker}_bad"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].sql, bad);

    // The browse path logs what it built; the cancel path logs its kill.
    let big = format!("-- {marker}_long\nSELECT {}", "1,".repeat(30_000) + "1");
    let seq = newest_seq();
    let _ = c.events(&big).await;
    let found = mentioning(seq, &format!("{marker}_long"));
    assert_eq!(found.len(), 1);
    assert!(
        found[0].sql.contains("more characters not shown"),
        "long statements are clipped"
    );
    assert!(found[0].sql.chars().count() < 20_100);
}
each_engine!(queries_are_logged_as_sent, sql);

async fn imports_log_their_batches(c: &Ctx) {
    let (int, text) = match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(100)"),
    };
    let t = c.scratch("log_import");
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY, v {text})",
        c.qt(&t)
    ))
    .await;
    let seq = newest_seq();
    let rows: Vec<Vec<Option<String>>> = (1..=1200)
        .map(|i| vec![Some(i.to_string()), Some(format!("import row {i}"))])
        .collect();
    assert_eq!(
        c.driver
            .insert_rows(&c.tref(&t), &["id".into(), "v".into()], rows)
            .await
            .unwrap(),
        1200
    );
    let mine = mentioning(seq, &t);
    let inserts: Vec<&LogEntry> = mine
        .iter()
        .filter(|e| e.sql.starts_with("INSERT INTO"))
        .collect();
    assert_eq!(
        inserts.len(),
        3,
        "1200 rows go in batches of at most 500: {}",
        inserts.len()
    );
    assert!(inserts[0].sql.contains("import row 1'"));
    assert!(inserts[2].sql.contains("import row 1200'"));
    let all = querylog::since(seq);
    assert!(
        all.iter()
            .any(|e| e.sql == "BEGIN" && e.seq < inserts[0].seq)
    );
    assert!(
        all.iter()
            .any(|e| e.sql == "COMMIT" && e.seq > inserts[2].seq)
    );
}
each_engine!(imports_log_their_batches, sql);

#[tokio::test]
async fn the_log_keeps_only_the_newest_entries() {
    // Plenty more statements than the log holds (SQLite is quick enough to do it here).
    let driver = connect_driver(Engine::Sqlite, Some(":memory:"), |_| {})
        .await
        .unwrap();
    let first = format!("SELECT 'capacity-first-{}'", uuid::Uuid::new_v4().simple());
    drain(driver.query(None, &first).await.unwrap())
        .await
        .unwrap();
    for i in 0..querylog::CAPACITY + 100 {
        drain(driver.query(None, &format!("SELECT {i}")).await.unwrap())
            .await
            .unwrap();
    }
    let last = format!("SELECT 'capacity-last-{}'", uuid::Uuid::new_v4().simple());
    drain(driver.query(None, &last).await.unwrap())
        .await
        .unwrap();
    let kept = querylog::since(0);
    assert!(kept.len() <= querylog::CAPACITY, "{} entries", kept.len());
    assert!(kept.iter().any(|e| e.sql == last));
    assert!(
        !kept.iter().any(|e| e.sql == first),
        "the oldest entry fell off"
    );
    assert!(kept.windows(2).all(|w| w[0].seq < w[1].seq));
    // Sequence numbers kept counting while entries fell off the front.
    assert!(kept[0].seq > 1);
    driver.close().await;
}
