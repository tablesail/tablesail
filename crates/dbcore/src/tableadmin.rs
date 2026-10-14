//! Drop, truncate and rename a table (or view, or collection) from the sidebar.
//!
//! [`plan`] turns the request into the statement(s) for one engine; [`table_admin`] looks the
//! object up (a view needs `DROP VIEW`, not `DROP TABLE`), plans, and - unless it is only a
//! preview - runs the statements through the driver, so they show up in the query log and the
//! driver forgets what it had cached about the table.

use crate::Engine;
use crate::dialect::Dialect;
use crate::driver::Driver;
use crate::drivers::mongo::admin_command;
use crate::error::{Error, Result};
use crate::model::{TableAction, TableKind, TableRef};

/// Statements for `action` on `table`, which the server reported to be a `kind`. SQL statements
/// end with `;`, MongoDB commands are shell calls without it.
pub fn plan(
    engine: Engine,
    dialect: Dialect,
    kind: TableKind,
    table: &TableRef,
    action: &TableAction,
) -> Result<Vec<String>> {
    if kind == TableKind::System {
        return Err(Error::InvalidInput(
            "System tables can't be changed.".to_owned(),
        ));
    }
    let new_name = match action {
        TableAction::Rename { new_name } => Some(checked_new_name(table, new_name)?),
        _ => None,
    };
    let name = display_kind(engine, kind);

    if engine == Engine::Mongodb {
        return plan_mongo(kind, name, table, action, new_name);
    }

    let target = dialect.qualified(table);
    let statement = match action {
        TableAction::Drop => format!("DROP {} {target};", object_keyword(engine, kind, name)?),
        TableAction::Truncate => {
            if !holds_rows(kind) {
                return Err(Error::InvalidInput(format!(
                    "A {name} has no rows of its own to remove."
                )));
            }
            match engine {
                Engine::Sqlite => format!("DELETE FROM {target};"),
                _ => format!("TRUNCATE TABLE {target};"),
            }
        }
        TableAction::Rename { .. } => {
            let new_name = new_name.unwrap_or_default();
            match engine {
                Engine::Mysql | Engine::Mariadb => {
                    let renamed = TableRef {
                        name: new_name,
                        ..table.clone()
                    };
                    format!("RENAME TABLE {target} TO {};", dialect.qualified(&renamed))
                }
                Engine::Sqlite if kind == TableKind::View => {
                    return Err(Error::Unsupported(
                        "SQLite can't rename a view. Drop it and create it again under the new name."
                            .to_owned(),
                    ));
                }
                _ => format!(
                    "ALTER {} {target} RENAME TO {};",
                    object_keyword(engine, kind, name)?,
                    dialect.quote_ident(&new_name)
                ),
            }
        }
    };
    Ok(vec![statement])
}

fn plan_mongo(
    kind: TableKind,
    name: &str,
    table: &TableRef,
    action: &TableAction,
    new_name: Option<String>,
) -> Result<Vec<String>> {
    match action {
        TableAction::Drop => {}
        TableAction::Truncate if holds_rows(kind) => {}
        TableAction::Truncate => {
            return Err(Error::InvalidInput(format!(
                "A MongoDB {name} has no documents of its own to remove."
            )));
        }
        TableAction::Rename { .. } if matches!(kind, TableKind::View | TableKind::TimeSeries) => {
            return Err(Error::Unsupported(format!(
                "MongoDB can't rename a {name}. Drop it and create it again under the new name."
            )));
        }
        TableAction::Rename { .. } => {}
    }
    let action = match new_name {
        Some(new_name) => TableAction::Rename { new_name },
        None => action.clone(),
    };
    Ok(vec![admin_command(&table.name, &action)])
}

/// What the user sees in messages ("view", "collection"...).
fn display_kind(engine: Engine, kind: TableKind) -> &'static str {
    match kind {
        TableKind::View => "view",
        TableKind::MaterializedView => "materialized view",
        TableKind::ForeignTable => "foreign table",
        TableKind::Sequence => "sequence",
        TableKind::Virtual => "virtual table",
        TableKind::TimeSeries => "time series collection",
        TableKind::Collection => "collection",
        TableKind::System => "system table",
        TableKind::Table if engine == Engine::Mongodb => "collection",
        TableKind::Table => "table",
    }
}

/// Objects that have rows to empty.
fn holds_rows(kind: TableKind) -> bool {
    matches!(
        kind,
        TableKind::Table | TableKind::Virtual | TableKind::Collection | TableKind::TimeSeries
    )
}

/// The keyword that follows `DROP` / `ALTER` for this kind of object.
fn object_keyword(engine: Engine, kind: TableKind, name: &str) -> Result<&'static str> {
    Ok(match (engine, kind) {
        (_, TableKind::Table | TableKind::Virtual) => "TABLE",
        (_, TableKind::View) => "VIEW",
        (Engine::Postgres, TableKind::MaterializedView) => "MATERIALIZED VIEW",
        (Engine::Postgres, TableKind::ForeignTable) => "FOREIGN TABLE",
        (Engine::Postgres | Engine::Mysql | Engine::Mariadb, TableKind::Sequence) => "SEQUENCE",
        _ => {
            return Err(Error::Unsupported(format!(
                "A {name} can't be changed here."
            )));
        }
    })
}

/// The trimmed new name, refusing one that is empty, unchanged or can't be an identifier.
fn checked_new_name(table: &TableRef, new_name: &str) -> Result<String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidInput("Enter a new name.".to_owned()));
    }
    if trimmed.contains('\0') {
        return Err(Error::InvalidInput(
            "A name can't contain a NUL character.".to_owned(),
        ));
    }
    if trimmed == table.name {
        return Err(Error::InvalidInput(
            "The new name is the same as the current one.".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

/// Looks `table` up in its database/schema; the kind decides which statement is right.
async fn kind_of(driver: &dyn Driver, table: &TableRef) -> Result<TableKind> {
    let tables = driver
        .list_tables(&table.database, table.schema.as_deref())
        .await?;
    tables
        .iter()
        .find(|t| t.table.name == table.name)
        .map(|t| t.kind)
        .ok_or_else(|| {
            Error::NotFound(match &table.schema {
                Some(schema) => format!("{schema}.{}", table.name),
                None => table.name.clone(),
            })
        })
}

/// Plans `action` and returns its statements; with `preview` false it also runs them.
/// Callers check that the connection may write.
pub async fn table_admin(
    driver: &dyn Driver,
    table: &TableRef,
    action: &TableAction,
    preview: bool,
) -> Result<Vec<String>> {
    let kind = kind_of(driver, table).await?;
    let statements = plan(driver.engine(), driver.dialect(), kind, table, action)?;
    if preview {
        return Ok(statements);
    }
    for statement in &statements {
        run(driver, &table.database, statement)
            .await
            .map_err(|e| e.with_statement(statement))?;
    }
    Ok(statements)
}

async fn run(driver: &dyn Driver, database: &str, statement: &str) -> Result<()> {
    let mut cursor = driver.query(Some(database), statement).await?;
    while cursor.next(1000).await?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: Dialect = Dialect::Postgres;
    const MY: Dialect = Dialect::Mysql {
        backslash_escapes: true,
    };

    fn table(database: &str, schema: Option<&str>, name: &str) -> TableRef {
        TableRef {
            database: database.to_owned(),
            schema: schema.map(str::to_owned),
            name: name.to_owned(),
        }
    }

    fn rename(to: &str) -> TableAction {
        TableAction::Rename {
            new_name: to.to_owned(),
        }
    }

    fn one(
        engine: Engine,
        dialect: Dialect,
        kind: TableKind,
        t: &TableRef,
        a: &TableAction,
    ) -> String {
        let mut statements = plan(engine, dialect, kind, t, a).unwrap();
        assert_eq!(statements.len(), 1);
        statements.remove(0)
    }

    #[test]
    fn actions_have_the_json_shape_the_ui_sends() {
        let parse = |json: &str| serde_json::from_str::<TableAction>(json).unwrap();
        assert_eq!(parse(r#"{"kind":"drop"}"#), TableAction::Drop);
        assert_eq!(parse(r#"{"kind":"truncate"}"#), TableAction::Truncate);
        assert_eq!(parse(r#"{"kind":"rename","newName":"b"}"#), rename("b"));
        assert_eq!(
            serde_json::to_string(&rename("b")).unwrap(),
            r#"{"kind":"rename","newName":"b"}"#
        );
        assert!(serde_json::from_str::<TableAction>(r#"{"kind":"rename"}"#).is_err());
        assert!(serde_json::from_str::<TableAction>(r#"{"kind":"vacuum"}"#).is_err());
    }

    #[test]
    fn postgres_picks_the_keyword_of_the_object() {
        let t = table("shop", Some("sales"), "orders");
        let plan = |kind, action: &TableAction| one(Engine::Postgres, PG, kind, &t, action);
        use TableKind::*;
        assert_eq!(
            plan(Table, &TableAction::Drop),
            "DROP TABLE \"sales\".\"orders\";"
        );
        assert_eq!(
            plan(View, &TableAction::Drop),
            "DROP VIEW \"sales\".\"orders\";"
        );
        assert_eq!(
            plan(MaterializedView, &TableAction::Drop),
            "DROP MATERIALIZED VIEW \"sales\".\"orders\";"
        );
        assert_eq!(
            plan(ForeignTable, &TableAction::Drop),
            "DROP FOREIGN TABLE \"sales\".\"orders\";"
        );
        assert_eq!(
            plan(Table, &TableAction::Truncate),
            "TRUNCATE TABLE \"sales\".\"orders\";"
        );
        assert_eq!(
            plan(Table, &rename("order \"lines\"")),
            "ALTER TABLE \"sales\".\"orders\" RENAME TO \"order \"\"lines\"\"\";"
        );
        assert_eq!(
            plan(View, &rename("v2")),
            "ALTER VIEW \"sales\".\"orders\" RENAME TO \"v2\";"
        );
        assert_eq!(
            plan(MaterializedView, &rename("m2")),
            "ALTER MATERIALIZED VIEW \"sales\".\"orders\" RENAME TO \"m2\";"
        );
    }

    #[test]
    fn mysql_renames_inside_the_same_database() {
        let t = table("shop", None, "orders");
        assert_eq!(
            one(Engine::Mysql, MY, TableKind::Table, &t, &TableAction::Drop),
            "DROP TABLE `shop`.`orders`;"
        );
        assert_eq!(
            one(Engine::Mariadb, MY, TableKind::View, &t, &TableAction::Drop),
            "DROP VIEW `shop`.`orders`;"
        );
        assert_eq!(
            one(
                Engine::Mysql,
                MY,
                TableKind::Table,
                &t,
                &TableAction::Truncate
            ),
            "TRUNCATE TABLE `shop`.`orders`;"
        );
        assert_eq!(
            one(Engine::Mysql, MY, TableKind::Table, &t, &rename("a`b")),
            "RENAME TABLE `shop`.`orders` TO `shop`.`a``b`;"
        );
        // Views are renamed the same way.
        assert_eq!(
            one(Engine::Mysql, MY, TableKind::View, &t, &rename("v")),
            "RENAME TABLE `shop`.`orders` TO `shop`.`v`;"
        );
    }

    #[test]
    fn sqlite_truncates_with_delete_and_cannot_rename_a_view() {
        let main = table("main", None, "t");
        let attached = table("aux", None, "t");
        let lite =
            |kind, t: &TableRef, a: &TableAction| one(Engine::Sqlite, Dialect::Sqlite, kind, t, a);
        assert_eq!(
            lite(TableKind::Table, &main, &TableAction::Drop),
            "DROP TABLE \"t\";"
        );
        assert_eq!(
            lite(TableKind::View, &attached, &TableAction::Drop),
            "DROP VIEW \"aux\".\"t\";"
        );
        assert_eq!(
            lite(TableKind::Table, &main, &TableAction::Truncate),
            "DELETE FROM \"t\";"
        );
        assert_eq!(
            lite(TableKind::Table, &attached, &rename("u")),
            "ALTER TABLE \"aux\".\"t\" RENAME TO \"u\";"
        );
        assert_eq!(
            lite(TableKind::Virtual, &main, &TableAction::Drop),
            "DROP TABLE \"t\";"
        );
        assert!(matches!(
            plan(
                Engine::Sqlite,
                Dialect::Sqlite,
                TableKind::View,
                &main,
                &rename("v")
            ),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            plan(
                Engine::Sqlite,
                Dialect::Sqlite,
                TableKind::View,
                &main,
                &TableAction::Truncate
            ),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn mongo_commands_read_like_the_shell() {
        let t = table("shop", None, "orders");
        let odd = table("shop", None, "my-coll");
        let mongo =
            |t: &TableRef, kind, a: &TableAction| one(Engine::Mongodb, Dialect::Mongo, kind, t, a);
        assert_eq!(
            mongo(&t, TableKind::Collection, &TableAction::Drop),
            "db.orders.drop()"
        );
        assert_eq!(
            mongo(&t, TableKind::Collection, &TableAction::Truncate),
            "db.orders.deleteMany({})"
        );
        assert_eq!(
            mongo(&t, TableKind::Collection, &rename("done")),
            "db.orders.renameCollection(\"done\")"
        );
        assert_eq!(
            mongo(&odd, TableKind::Collection, &TableAction::Drop),
            "db.getCollection(\"my-coll\").drop()"
        );
        assert!(
            plan(
                Engine::Mongodb,
                Dialect::Mongo,
                TableKind::View,
                &t,
                &TableAction::Truncate
            )
            .is_err()
        );
        assert!(
            plan(
                Engine::Mongodb,
                Dialect::Mongo,
                TableKind::View,
                &t,
                &rename("x")
            )
            .is_err()
        );
        assert_eq!(
            mongo(&t, TableKind::View, &TableAction::Drop),
            "db.orders.drop()"
        );
    }

    #[test]
    fn refuses_what_makes_no_sense() {
        let t = table("shop", Some("public"), "orders");
        let err = |kind, a: &TableAction| plan(Engine::Postgres, PG, kind, &t, a).unwrap_err();
        // Views and materialized views have no rows to truncate.
        assert!(matches!(
            err(TableKind::View, &TableAction::Truncate),
            Error::InvalidInput(m) if m.contains("view")
        ));
        assert!(matches!(
            err(TableKind::MaterializedView, &TableAction::Truncate),
            Error::InvalidInput(_)
        ));
        // System objects are left alone.
        assert!(matches!(
            err(TableKind::System, &TableAction::Drop),
            Error::InvalidInput(m) if m.contains("System")
        ));
        // A new name must be there, different, and sane.
        for bad in ["", "   ", "orders", " orders "] {
            assert!(
                matches!(err(TableKind::Table, &rename(bad)), Error::InvalidInput(_)),
                "{bad:?}"
            );
        }
        assert!(matches!(
            err(TableKind::Table, &rename("a\0b")),
            Error::InvalidInput(_)
        ));
        // The name is trimmed, a change of case counts as a change.
        assert_eq!(
            one(
                Engine::Postgres,
                PG,
                TableKind::Table,
                &t,
                &rename("  lines ")
            ),
            "ALTER TABLE \"public\".\"orders\" RENAME TO \"lines\";"
        );
        assert!(
            plan(
                Engine::Postgres,
                PG,
                TableKind::Table,
                &t,
                &rename("Orders")
            )
            .is_ok()
        );
    }

    #[test]
    fn hostile_names_stay_inside_their_quotes() {
        let t = table("db", Some("s\"x"), "t\"; DROP TABLE users; --");
        assert_eq!(
            one(
                Engine::Postgres,
                PG,
                TableKind::Table,
                &t,
                &TableAction::Drop
            ),
            "DROP TABLE \"s\"\"x\".\"t\"\"; DROP TABLE users; --\";"
        );
        let m = table("d`b", None, "t`; DROP TABLE users; --");
        assert_eq!(
            one(
                Engine::Mysql,
                MY,
                TableKind::Table,
                &m,
                &TableAction::Truncate
            ),
            "TRUNCATE TABLE `d``b`.`t``; DROP TABLE users; --`;"
        );
    }
}
