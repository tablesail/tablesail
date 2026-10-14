//! Drop, truncate and rename from the sidebar, against every engine: the right statement for
//! the kind of object, nothing happens on a preview, caches don't serve a table that is gone
//! or renamed, and bad requests fail without touching anything.

#[macro_use]
mod common;

use common::*;
use dbcore::Error;
use dbcore::model::{TableAction, TableKind, TableRef};
use dbcore::querylog;
use dbcore::tableadmin::table_admin;

async fn admin(
    c: &Ctx,
    table: &TableRef,
    action: TableAction,
    preview: bool,
) -> dbcore::Result<Vec<String>> {
    table_admin(c.driver.as_ref(), table, &action, preview).await
}

fn rename(to: &str) -> TableAction {
    TableAction::Rename {
        new_name: to.to_owned(),
    }
}

/// The tables of the test's database/schema, by name.
async fn listed(c: &Ctx) -> Vec<(String, TableKind)> {
    c.driver
        .list_tables(&c.database, c.schema.as_deref())
        .await
        .unwrap()
        .into_iter()
        .map(|t| (t.table.name, t.kind))
        .collect()
}

async fn kind_of(c: &Ctx, name: &str) -> Option<TableKind> {
    listed(c)
        .await
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, k)| k)
}

/// A name for a table to be renamed to, dropped at the end of the test.
fn target(c: &Ctx, suffix: &str) -> String {
    let name = c.name(suffix);
    if c.is_mongo() {
        c.cleanup_collection(&name);
    } else {
        c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&name)));
    }
    name
}

// ---------------------------------------------------------------------------
// SQL engines
// ---------------------------------------------------------------------------

async fn previews_change_nothing(c: &Ctx) {
    let t = c.make_items("preview", 10).await;
    let table = c.tref(&t);
    let new_name = c.name("preview_new");

    let drop = admin(c, &table, TableAction::Drop, true).await.unwrap();
    assert_eq!(drop, [format!("DROP TABLE {};", c.qt(&t))]);
    let truncate = admin(c, &table, TableAction::Truncate, true).await.unwrap();
    let expected = if c.is_sqlite() {
        format!("DELETE FROM {};", c.qt(&t))
    } else {
        format!("TRUNCATE TABLE {};", c.qt(&t))
    };
    assert_eq!(truncate, [expected]);
    let renamed = admin(c, &table, rename(&new_name), true).await.unwrap();
    assert_eq!(renamed.len(), 1);
    assert!(
        renamed[0].contains(&c.qt(&t)) && renamed[0].contains(&c.q(&new_name)),
        "{renamed:?}"
    );
    if c.is_my() {
        assert!(renamed[0].starts_with("RENAME TABLE "), "{renamed:?}");
        assert!(
            renamed[0].contains(&c.qt(&new_name)),
            "same database: {renamed:?}"
        );
    } else {
        assert!(renamed[0].starts_with("ALTER TABLE "), "{renamed:?}");
    }

    // The table is as it was.
    assert_eq!(c.side_count(&t).await, 10);
    assert_eq!(kind_of(c, &t).await, Some(TableKind::Table));
    assert_eq!(kind_of(c, &new_name).await, None);
}
each_engine!(previews_change_nothing, sql);

async fn drop_removes_the_table(c: &Ctx) {
    let t = c.make_items("drop", 5).await;
    let table = c.tref(&t);
    // Fill the driver's caches, which must not outlive the table.
    c.driver.table_structure(&table).await.unwrap();
    let browse = c.browse(&t);
    assert_eq!(c.page_ids(&browse, 0, 10).await, [1, 2, 3, 4, 5]);

    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    let statements = admin(c, &table, TableAction::Drop, false).await.unwrap();
    assert_eq!(statements, [format!("DROP TABLE {};", c.qt(&t))]);
    assert_eq!(kind_of(c, &t).await, None);
    assert!(
        matches!(
            c.driver.table_structure(&table).await,
            Err(Error::NotFound(_))
        ),
        "the cached structure is gone with the table"
    );
    assert!(c.driver.browse_page(&browse, 0, 10).await.is_err());
    assert!(
        c.try_side_exec(&format!("SELECT 1 FROM {}", c.qt(&t)))
            .await
            .is_err()
    );
    // It went through the driver, so the console has it.
    assert!(
        querylog::since(seq).iter().any(|e| e.sql == statements[0]),
        "the DROP is in the query log"
    );
    // Dropping what is not there is a clear error, not a silent success.
    assert!(matches!(
        admin(c, &table, TableAction::Drop, false).await,
        Err(Error::NotFound(_))
    ));
}
each_engine!(drop_removes_the_table, sql);

async fn truncate_empties_the_table_and_keeps_it(c: &Ctx) {
    let t = c.make_items("truncate", 25).await;
    let table = c.tref(&t);
    let before = c.driver.table_structure(&table).await.unwrap();
    assert_eq!(c.side_count(&t).await, 25);

    let statements = admin(c, &table, TableAction::Truncate, false)
        .await
        .unwrap();
    assert_eq!(statements.len(), 1);
    assert_eq!(c.side_count(&t).await, 0);
    assert_eq!(kind_of(c, &t).await, Some(TableKind::Table));
    let after = c.driver.table_structure(&table).await.unwrap();
    assert_eq!(after.columns, before.columns);
    assert_eq!(after.primary_key, before.primary_key);
    let page = c.page(&c.browse(&t), 0, 10).await;
    assert!(page.rows.is_empty());
    assert_eq!(
        c.driver.count_rows(&c.browse(&t)).await.unwrap().exact,
        Some(0)
    );
    // Usable again.
    c.insert_items(&t, 1..=3).await;
    assert_eq!(c.page_ids(&c.browse(&t), 0, 10).await, [1, 2, 3]);
}
each_engine!(truncate_empties_the_table_and_keeps_it, sql);

async fn rename_moves_the_table(c: &Ctx) {
    let t = c.make_items("rename", 12).await;
    let table = c.tref(&t);
    let first = target(c, "renamed");
    let before = c.driver.table_structure(&table).await.unwrap();
    c.page(&c.browse(&t), 0, 5).await;

    let statements = admin(c, &table, rename(&first), false).await.unwrap();
    assert_eq!(statements.len(), 1);
    assert_eq!(kind_of(c, &t).await, None);
    assert_eq!(kind_of(c, &first).await, Some(TableKind::Table));
    assert_eq!(c.side_count(&first).await, 12);
    assert!(
        matches!(
            c.driver.table_structure(&table).await,
            Err(Error::NotFound(_))
        ),
        "the old name is gone from the cache too"
    );
    let after = c.driver.table_structure(&c.tref(&first)).await.unwrap();
    assert_eq!(after.table.name, first);
    assert_eq!(after.columns, before.columns);
    assert_eq!(after.primary_key, before.primary_key);
    assert_eq!(c.page_ids(&c.browse(&first), 0, 20).await.len(), 12);

    // A name needing quotes survives them, and surrounding spaces are not part of it.
    let odd = format!("{} odd \"q\" ; -- x", c.prefix);
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&odd)));
    admin(c, &c.tref(&first), rename(&format!("  {odd} ")), false)
        .await
        .unwrap();
    assert_eq!(kind_of(c, &first).await, None);
    assert_eq!(kind_of(c, &odd).await, Some(TableKind::Table));
    assert_eq!(c.side_count(&odd).await, 12);
}
each_engine!(rename_moves_the_table, sql);

async fn views_are_dropped_and_renamed_as_views(c: &Ctx) {
    let t = c.make_items("view_base", 6).await;
    let v = c.scratch_view("view");
    c.side_exec(&format!(
        "CREATE VIEW {} AS SELECT id, name FROM {} WHERE id > 2",
        c.qt(&v),
        c.qt(&t)
    ))
    .await;
    let view = c.tref(&v);
    assert_eq!(kind_of(c, &v).await, Some(TableKind::View));

    assert_eq!(
        admin(c, &view, TableAction::Drop, true).await.unwrap(),
        [format!("DROP VIEW {};", c.qt(&v))]
    );
    // A view has no rows of its own.
    match admin(c, &view, TableAction::Truncate, false).await {
        Err(Error::InvalidInput(m)) => assert!(m.contains("view"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.side_count(&t).await, 6);

    let renamed = c.name("view_new");
    c.cleanup_sql(format!("DROP VIEW IF EXISTS {}", c.qt(&renamed)));
    if c.is_sqlite() {
        // SQLite has no way to rename a view.
        assert!(matches!(
            admin(c, &view, rename(&renamed), false).await,
            Err(Error::Unsupported(_))
        ));
        assert_eq!(kind_of(c, &v).await, Some(TableKind::View));
    } else {
        admin(c, &view, rename(&renamed), false).await.unwrap();
        assert_eq!(kind_of(c, &v).await, None);
        assert_eq!(kind_of(c, &renamed).await, Some(TableKind::View));
        assert_eq!(
            c.side_text(&format!("SELECT count(*) FROM {}", c.qt(&renamed)))
                .await
                .as_deref(),
            Some("4")
        );
    }
    let current = if c.is_sqlite() {
        v.clone()
    } else {
        renamed.clone()
    };
    admin(c, &c.tref(&current), TableAction::Drop, false)
        .await
        .unwrap();
    assert_eq!(kind_of(c, &current).await, None);
    // The base table is untouched.
    assert_eq!(c.side_count(&t).await, 6);
}
each_engine!(views_are_dropped_and_renamed_as_views, sql);

async fn bad_requests_fail_without_touching_anything(c: &Ctx) {
    let t = c.make_items("bad", 4).await;
    let other = c.make_items("bad_other", 2).await;
    let table = c.tref(&t);

    let spaced = format!(" {t} ");
    for name in ["", "   ", t.as_str(), spaced.as_str()] {
        assert!(
            matches!(
                admin(c, &table, rename(name), false).await,
                Err(Error::InvalidInput(_))
            ),
            "{name:?}"
        );
        assert!(matches!(
            admin(c, &table, rename(name), true).await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        admin(c, &table, rename("a\0b"), false).await,
        Err(Error::InvalidInput(_))
    ));
    // A table that is not there.
    let ghost = c.tref(&c.name("ghost"));
    for action in [TableAction::Drop, TableAction::Truncate, rename("x")] {
        assert!(matches!(
            admin(c, &ghost, action, false).await,
            Err(Error::NotFound(_))
        ));
    }

    // Renaming onto an existing table is the server's refusal, with the statement attached.
    match admin(c, &table, rename(&other), false).await {
        Err(Error::Database { statement, .. }) => {
            let statement = statement.expect("the failing statement is attached");
            assert!(statement.contains(&c.q(&other)), "{statement}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(c.side_count(&t).await, 4);
    assert_eq!(c.side_count(&other).await, 2);
    assert_eq!(kind_of(c, &t).await, Some(TableKind::Table));
}
each_engine!(bad_requests_fail_without_touching_anything, sql);

async fn foreign_keys_protect_the_parent(c: &Ctx) {
    // Registered in this order, so the child is dropped first at the end of the test.
    let (parent, child) = (c.scratch("fk_parent"), c.scratch("fk_child"));
    let (qp, qc) = (c.qt(&parent), c.qt(&child));
    let int = if c.is_pg() {
        "integer"
    } else if c.is_sqlite() {
        "INTEGER"
    } else {
        "INT"
    };
    c.side_exec(&format!("CREATE TABLE {qp} (id {int} PRIMARY KEY)"))
        .await;
    c.side_exec(&format!(
        "CREATE TABLE {qc} (id {int} PRIMARY KEY, p {int}, FOREIGN KEY (p) REFERENCES {qp} (id))"
    ))
    .await;
    c.side_exec(&format!("INSERT INTO {qp} VALUES (1), (2)"))
        .await;
    c.side_exec(&format!("INSERT INTO {qc} VALUES (10, 1)"))
        .await;
    let table = c.tref(&parent);

    // The server's refusal is passed on, with the statement that caused it, and nothing changes.
    // (Whether a truncate is allowed is up to the server, so either outcome is checked.)
    match admin(c, &table, TableAction::Truncate, false).await {
        Err(Error::Database {
            message, statement, ..
        }) => {
            assert!(!message.is_empty());
            assert!(statement.is_some());
            assert_eq!(c.side_count(&parent).await, 2, "the parent is intact");
        }
        Ok(_) => assert_eq!(c.side_count(&parent).await, 0),
        other => panic!("{other:?}"),
    }
    match admin(c, &table, TableAction::Drop, false).await {
        Err(Error::Database {
            message, statement, ..
        }) => {
            assert!(!message.is_empty());
            assert!(statement.is_some());
        }
        other => panic!("a referenced table can't be dropped: {other:?}"),
    }
    assert_eq!(kind_of(c, &parent).await, Some(TableKind::Table));
}
each_engine!(foreign_keys_protect_the_parent, sql);

/// Statements that look like attacks are just names.
async fn hostile_names_are_only_names(c: &Ctx) {
    let victim = c.make_items("victim", 3).await;
    let evil = format!("{}\"; DROP TABLE {victim}; --", c.prefix);
    // The names exist as tables, then are dropped and renamed like any other.
    c.cleanup_sql(format!("DROP TABLE IF EXISTS {}", c.qt(&evil)));
    c.side_exec(&format!(
        "CREATE TABLE {} (a {})",
        c.qt(&evil),
        if c.is_pg() {
            "integer"
        } else if c.is_sqlite() {
            "INTEGER"
        } else {
            "INT"
        }
    ))
    .await;
    let table = c.tref(&evil);
    assert_eq!(kind_of(c, &evil).await, Some(TableKind::Table));
    admin(c, &table, TableAction::Truncate, false)
        .await
        .unwrap();
    admin(c, &table, TableAction::Drop, false).await.unwrap();
    assert_eq!(kind_of(c, &evil).await, None);
    assert_eq!(c.side_count(&victim).await, 3, "nothing else was dropped");
}
each_engine!(hostile_names_are_only_names, sql);

// ---------------------------------------------------------------------------
// PostgreSQL: schemas and materialized views
// ---------------------------------------------------------------------------

async fn postgres_objects_in_other_schemas(c: &Ctx) {
    let schema = c.name("sch");
    c.cleanup_sql(format!("DROP SCHEMA IF EXISTS {} CASCADE", c.q(&schema)));
    c.side_exec(&format!("CREATE SCHEMA {}", c.q(&schema)))
        .await;
    let table = TableRef {
        database: c.database.clone(),
        schema: Some(schema.clone()),
        name: "items".to_owned(),
    };
    let qualified = c.dialect.qualified(&table);
    c.side_exec(&format!(
        "CREATE TABLE {qualified} (id integer PRIMARY KEY)"
    ))
    .await;
    c.side_exec(&format!("INSERT INTO {qualified} VALUES (1), (2)"))
        .await;
    let names = || async {
        c.driver
            .list_tables(&c.database, Some(&schema))
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.table.name)
            .collect::<Vec<_>>()
    };

    admin(c, &table, rename("moved"), false).await.unwrap();
    assert_eq!(names().await, ["moved"], "renamed in its own schema");
    let moved = TableRef {
        name: "moved".to_owned(),
        ..table.clone()
    };
    admin(c, &moved, TableAction::Truncate, false)
        .await
        .unwrap();
    assert_eq!(
        c.side_text(&format!(
            "SELECT count(*) FROM {}",
            c.dialect.qualified(&moved)
        ))
        .await
        .as_deref(),
        Some("0")
    );
    admin(c, &moved, TableAction::Drop, false).await.unwrap();
    assert!(names().await.is_empty());
}
each_engine!(postgres_objects_in_other_schemas, postgres);

async fn postgres_materialized_views(c: &Ctx) {
    let t = c.make_items("mv_base", 8).await;
    let mv = c.name("mv");
    let renamed = c.name("mv_new");
    c.cleanup_sql(format!(
        "DROP MATERIALIZED VIEW IF EXISTS {}",
        c.qt(&renamed)
    ));
    c.cleanup_sql(format!("DROP MATERIALIZED VIEW IF EXISTS {}", c.qt(&mv)));
    c.side_exec(&format!(
        "CREATE MATERIALIZED VIEW {} AS SELECT id, name FROM {}",
        c.qt(&mv),
        c.qt(&t)
    ))
    .await;
    let view = c.tref(&mv);
    assert_eq!(kind_of(c, &mv).await, Some(TableKind::MaterializedView));

    assert_eq!(
        admin(c, &view, TableAction::Drop, true).await.unwrap(),
        [format!("DROP MATERIALIZED VIEW {};", c.qt(&mv))]
    );
    assert!(matches!(
        admin(c, &view, TableAction::Truncate, false).await,
        Err(Error::InvalidInput(_))
    ));
    let statements = admin(c, &view, rename(&renamed), false).await.unwrap();
    assert!(
        statements[0].starts_with("ALTER MATERIALIZED VIEW "),
        "{statements:?}"
    );
    assert_eq!(kind_of(c, &mv).await, None);
    assert_eq!(
        kind_of(c, &renamed).await,
        Some(TableKind::MaterializedView)
    );
    admin(c, &c.tref(&renamed), TableAction::Drop, false)
        .await
        .unwrap();
    assert_eq!(kind_of(c, &renamed).await, None);
    assert_eq!(c.side_count(&t).await, 8);
}
each_engine!(postgres_materialized_views, postgres);

// ---------------------------------------------------------------------------
// MongoDB
// ---------------------------------------------------------------------------

async fn mongo_collections(c: &Ctx) {
    use mongodb::bson::doc;
    let name = c.scratch("coll");
    let table = c.tref(&name);
    let docs: Vec<_> = (1..=7)
        .map(|i| doc! { "n": i, "s": format!("doc {i}") })
        .collect();
    c.mongo_coll(&name).insert_many(docs).await.unwrap();
    assert_eq!(kind_of(c, &name).await, Some(TableKind::Collection));

    // Previews read like the shell and change nothing.
    assert_eq!(
        admin(c, &table, TableAction::Drop, true).await.unwrap(),
        [format!("db.{name}.drop()")]
    );
    assert_eq!(
        admin(c, &table, TableAction::Truncate, true).await.unwrap(),
        [format!("db.{name}.deleteMany({{}})")]
    );
    let renamed = target(c, "coll_new");
    assert_eq!(
        admin(c, &table, rename(&renamed), true).await.unwrap(),
        [format!("db.{name}.renameCollection(\"{renamed}\")")]
    );
    assert_eq!(c.side_count(&name).await, 7);

    // Truncate empties the collection but keeps it.
    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    admin(c, &table, TableAction::Truncate, false)
        .await
        .unwrap();
    assert_eq!(c.side_count(&name).await, 0);
    assert_eq!(kind_of(c, &name).await, Some(TableKind::Collection));
    assert!(
        querylog::since(seq)
            .iter()
            .any(|e| e.sql.contains("deleteMany") && e.sql.contains(&name)),
        "the command is in the query log"
    );
    c.mongo_coll(&name)
        .insert_many((1..=3).map(|i| doc! { "n": i }))
        .await
        .unwrap();

    // Rename moves the documents.
    admin(c, &table, rename(&renamed), false).await.unwrap();
    assert_eq!(kind_of(c, &name).await, None);
    assert_eq!(kind_of(c, &renamed).await, Some(TableKind::Collection));
    assert_eq!(c.side_count(&renamed).await, 3);

    // Renaming onto an existing collection is refused and changes nothing.
    let occupied = c.scratch("coll_occupied");
    c.mongo_coll(&occupied)
        .insert_one(doc! { "x": 1 })
        .await
        .unwrap();
    assert!(matches!(
        admin(c, &c.tref(&renamed), rename(&occupied), false).await,
        Err(Error::Database { .. })
    ));
    assert_eq!(c.side_count(&renamed).await, 3);
    assert_eq!(c.side_count(&occupied).await, 1);

    // Bad requests.
    for bad in ["", "  ", renamed.as_str()] {
        assert!(matches!(
            admin(c, &c.tref(&renamed), rename(bad), false).await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        admin(c, &c.tref(&c.name("ghost")), TableAction::Drop, false).await,
        Err(Error::NotFound(_))
    ));

    admin(c, &c.tref(&renamed), TableAction::Drop, false)
        .await
        .unwrap();
    assert_eq!(kind_of(c, &renamed).await, None);
    assert_eq!(c.side_count(&occupied).await, 1);
}
each_engine!(mongo_collections, mongo);

async fn mongo_names_that_need_quoting(c: &Ctx) {
    use mongodb::bson::doc;
    let name = c.scratch("odd-name.with dots");
    c.mongo_coll(&name)
        .insert_one(doc! { "a": 1 })
        .await
        .unwrap();
    let table = c.tref(&name);
    let statements = admin(c, &table, TableAction::Truncate, true).await.unwrap();
    assert_eq!(
        statements,
        [format!("db.getCollection(\"{name}\").deleteMany({{}})")]
    );

    let renamed = target(c, "odd-new.name");
    admin(c, &table, rename(&renamed), false).await.unwrap();
    assert_eq!(c.side_count(&renamed).await, 1);
    admin(c, &c.tref(&renamed), TableAction::Truncate, false)
        .await
        .unwrap();
    assert_eq!(c.side_count(&renamed).await, 0);
    admin(c, &c.tref(&renamed), TableAction::Drop, false)
        .await
        .unwrap();
    assert_eq!(kind_of(c, &renamed).await, None);
}
each_engine!(mongo_names_that_need_quoting, mongo);

async fn mongo_views(c: &Ctx) {
    use mongodb::bson::doc;
    let base = c.scratch("view_base");
    let view = c.scratch("view_on");
    c.mongo_coll(&base)
        .insert_many((1..=4).map(|i| doc! { "n": i }))
        .await
        .unwrap();
    c.mongo_db()
        .run_command(doc! { "create": &view, "viewOn": &base, "pipeline": [] })
        .await
        .unwrap();
    assert_eq!(kind_of(c, &view).await, Some(TableKind::View));
    let table = c.tref(&view);

    assert!(matches!(
        admin(c, &table, TableAction::Truncate, false).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        admin(c, &table, rename("renamed_view"), false).await,
        Err(Error::Unsupported(_))
    ));
    admin(c, &table, TableAction::Drop, false).await.unwrap();
    assert_eq!(kind_of(c, &view).await, None);
    assert_eq!(
        c.side_count(&base).await,
        4,
        "the collection under the view is untouched"
    );
}
each_engine!(mongo_views, mongo);
