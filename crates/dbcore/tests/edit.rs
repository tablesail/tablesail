//! Change sets: preview, apply in one transaction, rollback, keys, quoting.

#[macro_use]
mod common;

use std::time::{Duration, Instant};

use common::*;
use dbcore::model::{ChangeSet, ColumnInput, ColumnValue, FilterOp, RowChange};
use dbcore::value::CellInput;
use dbcore::{Engine, Error, Value};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn txt(s: &str) -> CellInput {
    CellInput::Text(s.to_owned())
}

fn inputs(pairs: &[(&str, CellInput)]) -> Vec<ColumnInput> {
    pairs
        .iter()
        .map(|(column, value)| ColumnInput {
            column: (*column).to_owned(),
            value: value.clone(),
        })
        .collect()
}

fn values(pairs: &[(&str, Value)]) -> Vec<ColumnValue> {
    pairs
        .iter()
        .map(|(column, value)| ColumnValue {
            column: (*column).to_owned(),
            value: value.clone(),
        })
        .collect()
}

fn ins(pairs: &[(&str, CellInput)]) -> RowChange {
    RowChange::Insert {
        values: inputs(pairs),
    }
}

fn upd(key: &[(&str, Value)], original: &[(&str, Value)], set: &[(&str, CellInput)]) -> RowChange {
    RowChange::Update {
        key: values(key),
        original: values(original),
        set: inputs(set),
    }
}

fn del(key: &[(&str, Value)]) -> RowChange {
    RowChange::Delete { key: values(key) }
}

fn id(i: i64) -> [(&'static str, Value); 1] {
    [("id", Value::Int(i))]
}

fn changes(c: &Ctx, table: &str, changes: Vec<RowChange>) -> ChangeSet {
    ChangeSet {
        table: c.tref(table),
        changes,
    }
}

fn types(c: &Ctx) -> (&'static str, &'static str) {
    match c.engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
        _ => ("INT", "VARCHAR(100)"),
    }
}

/// Whole table as `(id, name, grp, label)` rows through the side channel's view of the world.
async fn all(c: &Ctx, table: &str) -> Vec<Row> {
    c.rows(&format!("SELECT * FROM {} ORDER BY 1", c.qt(table)))
        .await
}

// ---------------------------------------------------------------------------
// Preview and apply
// ---------------------------------------------------------------------------

async fn preview_builds_sql_without_touching_the_table(c: &Ctx) {
    let t = c.make_items("preview", 5).await;
    let set = changes(
        c,
        &t,
        vec![
            ins(&[
                ("id", txt("100")),
                ("name", txt("O'Brien \"q\" \\ back")),
                ("grp", txt("1")),
                ("label", CellInput::Null),
            ]),
            upd(
                &id(1),
                &[("name", Value::Text("item 00001".into()))],
                &[("name", txt("renamed"))],
            ),
            del(&id(2)),
        ],
    );
    let sql = c.driver.preview_changes(&set).await.unwrap();
    assert_eq!(sql.len(), 3, "{sql:?}");
    let table = c.qt(&t);
    assert!(
        sql[0].starts_with(&format!("INSERT INTO {table}")),
        "{}",
        sql[0]
    );
    assert!(
        sql[0].contains("O''Brien"),
        "quotes are doubled: {}",
        sql[0]
    );
    assert!(sql[0].contains("NULL"));
    if c.is_my() {
        assert!(
            sql[0].contains("\\\\ back"),
            "backslashes are escaped on MySQL: {}",
            sql[0]
        );
    } else {
        assert!(sql[0].contains("\\ back"));
    }
    assert!(
        sql[1].starts_with(&format!("UPDATE {table} SET")),
        "{}",
        sql[1]
    );
    assert!(sql[1].contains("'renamed'"));
    assert!(sql[1].contains(&format!("{} = 1", c.q("id"))));
    assert!(
        sql[1].contains(&format!("{} = 'item 00001'", c.q("name"))),
        "{}",
        sql[1]
    );
    assert_eq!(
        sql[2],
        format!("DELETE FROM {table} WHERE {} = 2", c.q("id"))
    );
    // Nothing happened.
    assert_eq!(c.side_count(&t).await, 5);
    assert_eq!(all(c, &t).await, (1..=5).map(item_row).collect::<Vec<_>>());
    // An empty change set previews and applies to nothing.
    let empty = changes(c, &t, vec![]);
    assert!(c.driver.preview_changes(&empty).await.unwrap().is_empty());
    let applied = c.driver.apply_changes(&empty).await.unwrap();
    assert_eq!((applied.statements, applied.rows_affected), (0, 0));
}
each_engine!(preview_builds_sql_without_touching_the_table, sql);

async fn apply_runs_inserts_updates_and_deletes_together(c: &Ctx) {
    let t = c.make_items("apply", 10).await;
    let set = changes(
        c,
        &t,
        vec![
            ins(&[
                ("id", txt("11")),
                ("name", txt("eleven")),
                ("grp", txt("4")),
                ("label", txt("new")),
            ]),
            ins(&[
                ("id", txt("12")),
                ("name", txt("twelve")),
                ("grp", txt("5")),
                ("label", CellInput::Null),
            ]),
            upd(
                &id(1),
                &[
                    ("name", Value::Text("item 00001".into())),
                    ("grp", Value::Int(1)),
                ],
                &[("name", txt("one")), ("grp", txt("9"))],
            ),
            upd(
                &id(5),
                &[("label", Value::Null)],
                &[("label", txt("was null"))],
            ),
            del(&id(2)),
            del(&id(3)),
        ],
    );
    let applied = c.driver.apply_changes(&set).await.unwrap();
    assert_eq!(applied.statements, 6);
    assert_eq!(applied.rows_affected, 6);
    let rows = all(c, &t).await;
    let by_id = |i: i64| rows.iter().find(|r| r[0] == Value::Int(i)).cloned();
    assert_eq!(by_id(2), None);
    assert_eq!(by_id(3), None);
    assert_eq!(
        by_id(1),
        Some(vec![
            Value::Int(1),
            Value::Text("one".into()),
            Value::Int(9),
            Value::Text("lbl-1".into())
        ])
    );
    assert_eq!(by_id(5).unwrap()[3], Value::Text("was null".into()));
    assert_eq!(
        by_id(11),
        Some(vec![
            Value::Int(11),
            Value::Text("eleven".into()),
            Value::Int(4),
            Value::Text("new".into())
        ])
    );
    assert_eq!(by_id(12).unwrap()[3], Value::Null);
    assert_eq!(rows.len(), 10);
    // The page cache never hands back stale rows after an edit.
    let page = c.page(&c.browse(&t), 0, 3).await;
    assert_eq!(page.rows[0][1], Value::Text("one".into()));
}
each_engine!(apply_runs_inserts_updates_and_deletes_together, sql);

async fn defaults_nulls_and_auto_increment(c: &Ctx) {
    let t = c.scratch("defaults");
    let table = c.qt(&t);
    let ddl = match c.engine {
        Engine::Postgres => format!(
            "CREATE TABLE {table} (id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, name text NOT NULL DEFAULT 'anon', n integer DEFAULT 7, note text)"
        ),
        Engine::Sqlite => format!(
            "CREATE TABLE {table} (id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'anon', n INTEGER DEFAULT 7, note TEXT)"
        ),
        _ => format!(
            "CREATE TABLE {table} (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50) NOT NULL DEFAULT 'anon', n INT DEFAULT 7, note VARCHAR(50))"
        ),
    };
    c.side_exec(&ddl).await;
    let set = changes(
        c,
        &t,
        vec![
            ins(&[]),
            ins(&[("name", CellInput::Default), ("n", CellInput::Default)]),
            ins(&[
                ("name", txt("O'Brien \\ x")),
                ("n", CellInput::Null),
                ("note", txt("")),
            ]),
            ins(&[
                ("id", CellInput::Default),
                ("note", txt("explicit default id")),
            ]),
        ],
    );
    let sql = c.driver.preview_changes(&set).await.unwrap();
    assert_eq!(sql.len(), 4);
    assert!(
        sql[0].contains("DEFAULT VALUES") || sql[0].contains("() VALUES ()"),
        "{}",
        sql[0]
    );
    assert_eq!(c.driver.apply_changes(&set).await.unwrap().rows_affected, 4);
    let rows = all(c, &t).await;
    let anon = Value::Text("anon".into());
    assert_eq!(
        rows,
        vec![
            vec![Value::Int(1), anon.clone(), Value::Int(7), Value::Null],
            vec![Value::Int(2), anon.clone(), Value::Int(7), Value::Null],
            vec![
                Value::Int(3),
                Value::Text("O'Brien \\ x".into()),
                Value::Null,
                Value::Text("".into())
            ],
            vec![
                Value::Int(4),
                anon.clone(),
                Value::Int(7),
                Value::Text("explicit default id".into())
            ],
        ]
    );
    // An explicit key still works next to generated ones.
    c.driver
        .apply_changes(&changes(
            c,
            &t,
            vec![ins(&[("id", txt("50")), ("name", txt("fifty"))])],
        ))
        .await
        .unwrap();
    // Resetting a column to its default.
    let reset = changes(c, &t, vec![upd(&id(3), &[], &[("n", CellInput::Default)])]);
    if c.is_sqlite() {
        assert!(matches!(
            c.driver.preview_changes(&reset).await,
            Err(Error::InvalidInput(_))
        ));
    } else {
        c.driver.apply_changes(&reset).await.unwrap();
        assert_eq!(
            c.scalar(&format!("SELECT n FROM {table} WHERE id = 3"))
                .await,
            Value::Int(7)
        );
    }
}
each_engine!(defaults_nulls_and_auto_increment, sql);

async fn updating_the_primary_key_and_setting_the_same_value(c: &Ctx) {
    let t = c.make_items("pk_update", 5).await;
    // Change the key itself.
    c.driver
        .apply_changes(&changes(
            c,
            &t,
            vec![upd(&id(3), &[("id", Value::Int(3))], &[("id", txt("300"))])],
        ))
        .await
        .unwrap();
    let ids_now = ids(&all(c, &t).await, 0);
    assert_eq!(ids_now, [1, 2, 4, 5, 300]);

    // Writing the value that is already there is not a conflict (matched rows count).
    let same = changes(
        c,
        &t,
        vec![upd(
            &id(1),
            &[("name", Value::Text("item 00001".into()))],
            &[("name", txt("item 00001"))],
        )],
    );
    let applied = c.driver.apply_changes(&same).await.unwrap();
    assert_eq!(applied.rows_affected, 1, "{c:?}", c = c.engine);
    // Without any optimistic check at all.
    let blind = changes(c, &t, vec![upd(&id(2), &[], &[("grp", txt("6"))])]);
    assert_eq!(
        c.driver.apply_changes(&blind).await.unwrap().rows_affected,
        1
    );
    // Several columns at once, including NULL.
    let multi = changes(
        c,
        &t,
        vec![upd(
            &id(4),
            &[
                ("label", Value::Text("lbl-4".into())),
                ("grp", Value::Int(4)),
            ],
            &[
                ("label", CellInput::Null),
                ("grp", txt("0")),
                ("name", txt("multi")),
            ],
        )],
    );
    c.driver.apply_changes(&multi).await.unwrap();
    let row = all(c, &t)
        .await
        .into_iter()
        .find(|r| r[0] == Value::Int(4))
        .unwrap();
    assert_eq!(
        row,
        vec![
            Value::Int(4),
            Value::Text("multi".into()),
            Value::Int(0),
            Value::Null
        ]
    );
}
each_engine!(updating_the_primary_key_and_setting_the_same_value, sql);

// ---------------------------------------------------------------------------
// Optimistic concurrency
// ---------------------------------------------------------------------------

async fn stale_edits_are_refused_and_change_nothing(c: &Ctx) {
    let t = c.make_items("stale", 5).await;
    let table = c.qt(&t);
    // Someone else edits row 1 after the grid loaded it.
    c.side_exec(&format!(
        "UPDATE {table} SET name = 'changed elsewhere' WHERE id = 1"
    ))
    .await;
    let before = all(c, &t).await;
    let stale = changes(
        c,
        &t,
        vec![
            upd(&id(2), &[], &[("grp", txt("3"))]),
            upd(
                &id(1),
                &[("name", Value::Text("item 00001".into()))],
                &[("name", txt("mine"))],
            ),
            upd(&id(3), &[], &[("grp", txt("3"))]),
        ],
    );
    let err = c.driver.apply_changes(&stale).await.unwrap_err();
    match &err {
        Error::Database {
            message,
            statement: Some(statement),
            ..
        } => {
            assert_eq!(message, "Row was changed or deleted by someone else");
            assert!(
                statement.contains("'mine'") && statement.contains("item 00001"),
                "{statement}"
            );
        }
        other => panic!("{other:?}"),
    }
    // Row 2's earlier update was rolled back with the rest.
    assert_eq!(all(c, &t).await, before);

    // Unchanged-NULL originals are compared with IS NULL, and a non-NULL now is a conflict.
    let err = c
        .driver
        .apply_changes(&changes(
            c,
            &t,
            vec![upd(
                &id(2),
                &[("label", Value::Null)],
                &[("label", txt("x"))],
            )],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Database { .. }), "{err:?}");
    assert_eq!(all(c, &t).await, before);
    // ... while a row that really is NULL matches.
    c.driver
        .apply_changes(&changes(
            c,
            &t,
            vec![upd(
                &id(5),
                &[("label", Value::Null)],
                &[("label", txt("filled"))],
            )],
        ))
        .await
        .unwrap();

    // Deleting a row that is already gone.
    c.side_exec(&format!("DELETE FROM {table} WHERE id = 4"))
        .await;
    let before = all(c, &t).await;
    let err = c
        .driver
        .apply_changes(&changes(c, &t, vec![del(&id(3)), del(&id(4))]))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { message, .. } if message.contains("changed or deleted")),
        "{err:?}"
    );
    assert_eq!(
        all(c, &t).await,
        before,
        "the first delete is rolled back too"
    );

    // Updating a row that is already gone.
    let err = c
        .driver
        .apply_changes(&changes(
            c,
            &t,
            vec![upd(&id(4), &[], &[("grp", txt("1"))])],
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Database { .. }));
}
each_engine!(stale_edits_are_refused_and_change_nothing, sql);

async fn two_clients_racing_for_one_row_have_one_winner(c: &Ctx) {
    let t = c.make_items("race", 3).await;
    let mk = |new: &str| {
        changes(
            c,
            &t,
            vec![upd(
                &id(1),
                &[("name", Value::Text("item 00001".into()))],
                &[("name", txt(new))],
            )],
        )
    };
    let (a, b) = (mk("from a"), mk("from b"));
    let (ra, rb) = tokio::join!(c.driver.apply_changes(&a), c.driver.apply_changes(&b));
    let wins = [ra.is_ok(), rb.is_ok()].iter().filter(|w| **w).count();
    assert_eq!(wins, 1, "exactly one writer wins: {ra:?} / {rb:?}");
    let name = c
        .scalar(&format!("SELECT name FROM {} WHERE id = 1", c.qt(&t)))
        .await;
    assert!(
        name == Value::Text("from a".into()) || name == Value::Text("from b".into()),
        "{name:?}"
    );

    // Independent rows edited at the same time both land.
    let c1 = changes(c, &t, vec![upd(&id(2), &[], &[("grp", txt("10"))])]);
    let c2 = changes(c, &t, vec![upd(&id(3), &[], &[("grp", txt("11"))])]);
    let (r1, r2) = tokio::join!(c.driver.apply_changes(&c1), c.driver.apply_changes(&c2));
    r1.unwrap();
    r2.unwrap();
    let grp: Vec<Value> = all(c, &t).await.iter().map(|r| r[2].clone()).collect();
    assert_eq!(&grp[1..], [Value::Int(10), Value::Int(11)]);
}
each_engine!(two_clients_racing_for_one_row_have_one_winner, sql);

// ---------------------------------------------------------------------------
// Rollback
// ---------------------------------------------------------------------------

/// `rules(id PK, code UNIQUE NOT NULL, qty >= 0, parent_id -> parent)`, with two rows.
async fn rules_table(c: &Ctx) -> (String, String) {
    let (int, text) = types(c);
    let parent = c.scratch("parent");
    let rules = c.scratch("rules");
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY)",
        c.qt(&parent)
    ))
    .await;
    c.side_exec(&format!("INSERT INTO {} VALUES (1), (2)", c.qt(&parent)))
        .await;
    c.side_exec(&format!(
        "CREATE TABLE {} (id {int} PRIMARY KEY, code {text} NOT NULL UNIQUE, qty {int} CHECK (qty >= 0), parent_id {int}, FOREIGN KEY (parent_id) REFERENCES {} (id))",
        c.qt(&rules),
        c.qt(&parent)
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {} VALUES (1, 'a', 10, 1), (2, 'b', 20, 2)",
        c.qt(&rules)
    ))
    .await;
    (rules, parent)
}

async fn a_failing_statement_rolls_back_the_whole_set(c: &Ctx) {
    let (t, _) = rules_table(c).await;
    let before = all(c, &t).await;
    let new_row = |i: &str, code: &str| {
        ins(&[
            ("id", txt(i)),
            ("code", txt(code)),
            ("qty", txt("1")),
            ("parent_id", txt("1")),
        ])
    };
    let cases: Vec<(&str, Vec<RowChange>, &str)> = vec![
        (
            "unique violation in the middle",
            vec![new_row("10", "x"), new_row("11", "a"), new_row("12", "y")],
            "unique",
        ),
        (
            "unique violation on the last statement",
            vec![new_row("10", "x"), new_row("11", "y"), new_row("12", "b")],
            "unique",
        ),
        (
            "primary key violation after an update",
            vec![upd(&id(1), &[], &[("qty", txt("99"))]), new_row("2", "z")],
            "pk",
        ),
        (
            "unique violation caused by an update",
            vec![new_row("10", "x"), upd(&id(1), &[], &[("code", txt("b"))])],
            "unique",
        ),
        (
            "NOT NULL violation",
            vec![
                new_row("10", "x"),
                ins(&[("id", txt("11")), ("code", CellInput::Null)]),
            ],
            "notnull",
        ),
        (
            "check violation",
            vec![new_row("10", "x"), upd(&id(2), &[], &[("qty", txt("-5"))])],
            "check",
        ),
        (
            "foreign key violation",
            vec![
                new_row("10", "x"),
                ins(&[
                    ("id", txt("11")),
                    ("code", txt("fk")),
                    ("parent_id", txt("999")),
                ]),
            ],
            "fk",
        ),
        (
            "delete that matches nothing",
            vec![new_row("10", "x"), del(&id(77))],
            "missing",
        ),
        (
            "update after a delete of the same row",
            vec![del(&id(2)), upd(&id(2), &[], &[("qty", txt("1"))])],
            "missing",
        ),
    ];
    for (name, set, kind) in cases {
        let statements = set.len() as u64;
        let err = c
            .driver
            .apply_changes(&changes(c, &t, set))
            .await
            .expect_err(name);
        let Error::Database {
            message,
            code,
            statement,
        } = &err
        else {
            panic!("{name}: {err:?}");
        };
        assert!(
            statement.is_some(),
            "{name}: the failing statement is attached"
        );
        assert!(!message.is_empty(), "{name}");
        match (kind, c.engine) {
            ("unique", Engine::Postgres) => {
                assert_eq!(code.as_deref(), Some("23505"), "{name}: {err:?}")
            }
            ("pk", Engine::Postgres) => assert_eq!(code.as_deref(), Some("23505"), "{name}"),
            ("notnull", Engine::Postgres) => assert_eq!(code.as_deref(), Some("23502"), "{name}"),
            ("check", Engine::Postgres) => assert_eq!(code.as_deref(), Some("23514"), "{name}"),
            ("fk", Engine::Postgres) => assert_eq!(code.as_deref(), Some("23503"), "{name}"),
            ("unique" | "pk", Engine::Mysql | Engine::Mariadb) => {
                assert!(message.contains("Duplicate entry"), "{name}: {message}");
                assert_eq!(code.as_deref(), Some("23000"), "{name}");
            }
            ("missing", _) => assert_eq!(
                message, "Row was changed or deleted by someone else",
                "{name}"
            ),
            ("unique" | "pk", Engine::Sqlite) => {
                assert!(message.contains("UNIQUE"), "{name}: {message}")
            }
            _ => assert!(code.is_some() || kind == "missing", "{name}: {err:?}"),
        }
        let _ = statements;
        assert_eq!(
            all(c, &t).await,
            before,
            "{name}: the table must be unchanged"
        );
        assert_eq!(c.side_count(&t).await, 2);
    }
    // The driver is still fully usable after every failure.
    let ok = c
        .driver
        .apply_changes(&changes(
            c,
            &t,
            vec![ins(&[
                ("id", txt("20")),
                ("code", txt("fine")),
                ("qty", txt("5")),
            ])],
        ))
        .await
        .unwrap();
    assert_eq!(ok.rows_affected, 1);
    assert_eq!(c.side_count(&t).await, 3);
}
each_engine!(a_failing_statement_rolls_back_the_whole_set, sql);

async fn type_errors_roll_back(c: &Ctx) {
    // SQLite stores anything anywhere, so there is nothing to reject there.
    let (t, _) = rules_table(c).await;
    let before = all(c, &t).await;
    let set = changes(
        c,
        &t,
        vec![
            upd(&id(1), &[], &[("qty", txt("77"))]),
            upd(&id(2), &[], &[("qty", txt("not a number"))]),
        ],
    );
    let err = c.driver.apply_changes(&set).await.unwrap_err();
    assert!(
        matches!(&err, Error::Database { statement: Some(s), .. } if s.contains("not a number")),
        "{err:?}"
    );
    if c.is_pg() {
        assert!(
            matches!(&err, Error::Database { code: Some(code), .. } if code == "22P02"),
            "{err:?}"
        );
    }
    assert_eq!(all(c, &t).await, before);
}
each_engine!(type_errors_roll_back, postgres, mysql, mariadb);

async fn a_big_change_set_is_atomic_and_fast(c: &Ctx) {
    let t = c.make_items("big_set", 1000).await;
    let mut set = Vec::new();
    for i in 1001..=3000 {
        set.push(ins(&[
            ("id", txt(&i.to_string())),
            ("name", txt(&item(i).name)),
            ("grp", txt(&item(i).grp.to_string())),
        ]));
    }
    for i in 1..=300 {
        set.push(upd(&id(i), &[], &[("grp", txt("100"))]));
    }
    for i in 301..=400 {
        set.push(del(&id(i)));
    }
    let started = Instant::now();
    let applied = c
        .driver
        .apply_changes(&changes(c, &t, set.clone()))
        .await
        .unwrap();
    assert_eq!(applied.statements, 2400);
    assert_eq!(applied.rows_affected, 2400);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(c.side_count(&t).await, 1000 + 2000 - 100);
    assert_eq!(
        c.scalar(&format!(
            "SELECT COUNT(*) FROM {} WHERE grp = 100",
            c.qt(&t)
        ))
        .await,
        Value::Int(300)
    );

    // The same size of change set with a duplicate key at the very end changes nothing.
    let before = c.side_count(&t).await;
    let mut set = Vec::new();
    for i in 5001..=6500 {
        set.push(ins(&[
            ("id", txt(&i.to_string())),
            ("name", txt("x")),
            ("grp", txt("1")),
        ]));
    }
    set.push(ins(&[
        ("id", txt("1")),
        ("name", txt("duplicate")),
        ("grp", txt("1")),
    ]));
    assert!(c.driver.apply_changes(&changes(c, &t, set)).await.is_err());
    assert_eq!(c.side_count(&t).await, before);
}
each_engine!(a_big_change_set_is_atomic_and_fast, sql);

// ---------------------------------------------------------------------------
// Keys and identifiers
// ---------------------------------------------------------------------------

async fn composite_primary_keys(c: &Ctx) {
    let (int, text) = types(c);
    let t = c.scratch("composite");
    let table = c.qt(&t);
    c.side_exec(&format!(
        "CREATE TABLE {table} (tenant {int} NOT NULL, item {text} NOT NULL, qty {int}, PRIMARY KEY (item, tenant))"
    ))
    .await;
    c.side_exec(&format!(
        "INSERT INTO {table} VALUES (1, 'a', 10), (1, 'b', 20), (2, 'a', 30), (2, 'b', 40)"
    ))
    .await;
    let key = |tenant: i64, item: &str| {
        [
            ("tenant", Value::Int(tenant)),
            ("item", Value::Text(item.into())),
        ]
    };
    // Rows that share one key column are told apart by the other.
    let set = changes(
        c,
        &t,
        vec![
            upd(
                &key(2, "a"),
                &[("qty", Value::Int(30))],
                &[("qty", txt("31"))],
            ),
            del(&key(1, "b")),
            ins(&[("tenant", txt("3")), ("item", txt("c")), ("qty", txt("50"))]),
        ],
    );
    let sql = c.driver.preview_changes(&set).await.unwrap();
    assert!(
        sql[0].contains(&c.q("tenant")) && sql[0].contains(&c.q("item")),
        "{}",
        sql[0]
    );
    assert_eq!(c.driver.apply_changes(&set).await.unwrap().rows_affected, 3);
    let rows = c
        .rows(&format!(
            "SELECT tenant, item, qty FROM {table} ORDER BY tenant, item"
        ))
        .await;
    let ints = |r: &Row| (r[0].clone(), r[1].clone(), r[2].clone());
    assert_eq!(
        rows.iter().map(ints).collect::<Vec<_>>(),
        vec![
            (Value::Int(1), Value::Text("a".into()), Value::Int(10)),
            (Value::Int(2), Value::Text("a".into()), Value::Int(31)),
            (Value::Int(2), Value::Text("b".into()), Value::Int(40)),
            (Value::Int(3), Value::Text("c".into()), Value::Int(50)),
        ]
    );
    // A key that is missing one of its columns is refused, before anything runs.
    let partial = changes(c, &t, vec![del(&[("tenant", Value::Int(1))])]);
    assert!(matches!(
        c.driver.preview_changes(&partial).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        c.driver.apply_changes(&partial).await,
        Err(Error::InvalidInput(_))
    ));
    let empty_key = changes(c, &t, vec![del(&[])]);
    assert!(matches!(
        c.driver.apply_changes(&empty_key).await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(c.side_count(&t).await, 4);
    // Inserting an existing composite key fails; a new combination of existing parts works.
    let dup = changes(
        c,
        &t,
        vec![ins(&[
            ("tenant", txt("1")),
            ("item", txt("a")),
            ("qty", txt("1")),
        ])],
    );
    assert!(matches!(
        c.driver.apply_changes(&dup).await,
        Err(Error::Database { .. })
    ));
    let fresh = changes(
        c,
        &t,
        vec![ins(&[
            ("tenant", txt("1")),
            ("item", txt("b")),
            ("qty", txt("1")),
        ])],
    );
    c.driver.apply_changes(&fresh).await.unwrap();
    assert_eq!(c.side_count(&t).await, 5);
}
each_engine!(composite_primary_keys, sql);

async fn bad_change_sets_are_refused_before_running(c: &Ctx) {
    let t = c.make_items("bad_sets", 3).await;
    let before = all(c, &t).await;
    let bad: Vec<(&str, Vec<RowChange>)> = vec![
        (
            "unknown column in an insert",
            vec![ins(&[("nope", txt("1"))])],
        ),
        (
            "unknown column in an update",
            vec![upd(&id(1), &[], &[("nope", txt("1"))])],
        ),
        ("update without changes", vec![upd(&id(1), &[], &[])]),
        ("unknown key column", vec![del(&[("nope", Value::Int(1))])]),
        ("no key", vec![del(&[])]),
        (
            "bad base64",
            vec![ins(&[
                ("id", txt("9")),
                ("name", CellInput::Bytes("!!not base64!!".into())),
            ])],
        ),
    ];
    for (name, set) in bad {
        let set = changes(c, &t, set);
        assert!(
            matches!(
                c.driver.preview_changes(&set).await,
                Err(Error::InvalidInput(_))
            ),
            "{name}: preview"
        );
        assert!(
            matches!(
                c.driver.apply_changes(&set).await,
                Err(Error::InvalidInput(_))
            ),
            "{name}: apply"
        );
    }
    // A good change before a bad one is not applied either.
    let mixed = changes(
        c,
        &t,
        vec![
            upd(&id(1), &[], &[("grp", txt("5"))]),
            ins(&[("nope", txt("1"))]),
        ],
    );
    assert!(c.driver.apply_changes(&mixed).await.is_err());
    assert_eq!(all(c, &t).await, before);
    // Unknown table.
    let missing = changes(c, "no_such_table_xyz", vec![del(&id(1))]);
    assert!(matches!(
        c.driver.apply_changes(&missing).await,
        Err(Error::NotFound(_) | Error::Database { .. })
    ));
}
each_engine!(bad_change_sets_are_refused_before_running, sql);

async fn weird_identifiers_in_change_sets(c: &Ctx) {
    // Table and column names that need quoting, one of them the primary key.
    let fancy = if c.is_my() {
        "Ωméga ünï 日本"
    } else {
        "Ωméga ünï 🎉"
    };
    let table = format!("{}_w \"q\" 'x'; --{fancy} select", c.prefix);
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&table)));
    let (int, text) = types(c);
    let key_col = "Mixed \"Key\" select";
    let cols = [
        "order",
        "with space",
        "a'b",
        "semi;colon",
        "--dash",
        "100% _wild_",
        fancy,
    ];
    let defs: Vec<String> = std::iter::once(format!("{} {int} PRIMARY KEY", c.q(key_col)))
        .chain(cols.iter().map(|n| format!("{} {text}", c.q(n))))
        .collect();
    c.side_exec(&format!(
        "CREATE TABLE {} ({})",
        c.qt(&table),
        defs.join(", ")
    ))
    .await;

    let key = |i: i64| [(key_col, Value::Int(i))];
    let insert_row = |i: i64| {
        let mut pairs: Vec<(&str, CellInput)> = vec![(key_col, txt(&i.to_string()))];
        pairs.extend(cols.iter().map(|n| (*n, txt(&format!("v{i}")))));
        ins(&pairs)
    };
    let set = changes(c, &table, vec![insert_row(1), insert_row(2), insert_row(3)]);
    let sql = c.driver.preview_changes(&set).await.unwrap();
    assert!(sql[0].contains(&c.q(key_col)), "{}", sql[0]);
    assert!(
        sql[0].starts_with(&format!("INSERT INTO {}", c.qt(&table))),
        "{}",
        sql[0]
    );
    assert_eq!(c.driver.apply_changes(&set).await.unwrap().rows_affected, 3);

    // Update every weird column of row 2 (with the optimistic original), then delete row 3.
    for col in cols {
        let set = changes(
            c,
            &table,
            vec![upd(
                &key(2),
                &[(col, Value::Text("v2".into()))],
                &[(col, txt(&format!("edited {col}")))],
            )],
        );
        c.driver
            .apply_changes(&set)
            .await
            .unwrap_or_else(|e| panic!("updating `{col}`: {e:?}"));
    }
    c.driver
        .apply_changes(&changes(c, &table, vec![del(&key(3))]))
        .await
        .unwrap();
    let page = c.page(&c.browse(&table), 0, 10).await;
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0][1], Value::Text("v1".into()));
    for (i, col) in cols.iter().enumerate() {
        assert_eq!(
            page.rows[1][i + 1],
            Value::Text(format!("edited {col}")),
            "column {col}"
        );
    }
    // Filtering and sorting by the weird key column.
    let mut req = c.browse(&table);
    req.sort = vec![sort(key_col, true)];
    req.filters = vec![filter(key_col, FilterOp::Lte, Some("2"))];
    assert_eq!(c.page_ids(&req, 0, 10).await, [2, 1]);
}
each_engine!(weird_identifiers_in_change_sets, sql);

async fn values_of_many_types_can_be_inserted_as_text(c: &Ctx) {
    let t = c.scratch("typed_insert");
    let table = c.qt(&t);
    let ddl = match c.engine {
        Engine::Postgres => format!(
            "CREATE TABLE {table} (id integer PRIMARY KEY, n numeric(12,3), f double precision, b boolean, d date, ts timestamptz, j jsonb, raw bytea, u uuid, arr integer[])"
        ),
        Engine::Sqlite => format!(
            "CREATE TABLE {table} (id INTEGER PRIMARY KEY, n NUMERIC, f REAL, b BOOLEAN, d DATE, ts DATETIME, j JSON, raw BLOB)"
        ),
        _ => format!(
            "CREATE TABLE {table} (id INT PRIMARY KEY, n DECIMAL(12,3), f DOUBLE, b TINYINT(1), d DATE, ts DATETIME(3), j JSON, raw BLOB)"
        ),
    };
    c.side_exec(&ddl).await;
    let b_true = if c.is_pg() { "true" } else { "1" };
    let mut row = vec![
        ("id", txt("1")),
        ("n", txt("-12345.678")),
        ("f", txt("0.1")),
        ("b", txt(b_true)),
        ("d", txt("2024-02-29")),
        ("ts", txt("2024-02-29 13:14:15.123")),
        (
            "j",
            CellInput::Json(serde_json::json!({"a": [1, {"b": null}], "ü": "🎉 it's"})),
        ),
        ("raw", CellInput::Bytes("3q2+7w==".into())),
    ];
    if c.is_pg() {
        row.push(("u", txt("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11")));
        row.push(("arr", txt("{1,2,3}")));
    }
    c.driver
        .apply_changes(&changes(c, &t, vec![ins(&row), ins(&[("id", txt("2"))])]))
        .await
        .unwrap();
    let page = c.page(&c.browse(&t), 0, 10).await;
    let r = &page.rows[0];
    assert_eq!(r[0], Value::Int(1));
    match c.engine {
        Engine::Sqlite => assert_eq!(r[1], Value::Float(-12345.678)),
        _ => assert_eq!(r[1], Value::Decimal("-12345.678".into())),
    }
    assert_eq!(r[2], Value::Float(0.1));
    assert_eq!(r[3], Value::Bool(true));
    assert_eq!(
        r[4],
        if c.is_sqlite() {
            Value::Text("2024-02-29".into())
        } else {
            Value::Date("2024-02-29".into())
        }
    );
    match c.engine {
        Engine::Postgres => assert_eq!(r[5], Value::DateTime("2024-02-29 13:14:15.123+00".into())),
        Engine::Sqlite => assert_eq!(r[5], Value::Text("2024-02-29 13:14:15.123".into())),
        _ => assert_eq!(r[5], Value::DateTime("2024-02-29 13:14:15.123".into())),
    }
    if c.is_sqlite() || c.is_mariadb() {
        // SQLite and MariaDB (JSON is LONGTEXT there) keep the JSON as the text it was given.
        let Value::Text(s) = &r[6] else {
            panic!("{:?}", r[6])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(s).unwrap(),
            serde_json::json!({"a": [1, {"b": null}], "ü": "🎉 it's"})
        );
    } else {
        assert_eq!(
            r[6],
            Value::Json(serde_json::json!({"a": [1, {"b": null}], "ü": "🎉 it's"}))
        );
    }
    assert_eq!(r[7], Value::bytes(vec![0xde, 0xad, 0xbe, 0xef]));
    if c.is_pg() {
        assert_eq!(
            r[8],
            Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
        );
    }
    // The second row has NULLs everywhere.
    assert!(page.rows[1].iter().skip(1).all(Value::is_null));
}
each_engine!(values_of_many_types_can_be_inserted_as_text, sql);

async fn read_only_tables_cannot_be_edited(c: &Ctx) {
    let (int, text) = types(c);
    let t = c.scratch("no_pk");
    c.side_exec(&format!("CREATE TABLE {} (a {int}, b {text})", c.qt(&t)))
        .await;
    c.side_exec(&format!("INSERT INTO {} VALUES (1, 'x')", c.qt(&t)))
        .await;
    let set = changes(c, &t, vec![del(&[("a", Value::Int(1))])]);
    match c.driver.apply_changes(&set).await {
        Err(Error::InvalidInput(reason)) => assert!(reason.contains("primary key"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.side_count(&t).await, 1);
    // CSV import works on tables without a primary key (it only appends).
    let n = c
        .driver
        .insert_rows(
            &c.tref(&t),
            &["a".into(), "b".into()],
            vec![vec![Some("2".into()), Some("y".into())]],
        )
        .await
        .unwrap();
    assert_eq!(n, 1);
}
each_engine!(read_only_tables_cannot_be_edited, sql);
