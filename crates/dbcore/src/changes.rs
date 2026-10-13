//! Turns a `ChangeSet` into SQL statements (Postgres, MySQL, SQLite).

use crate::dialect::Dialect;
use crate::error::{Error, Result};
use crate::model::{
    ChangeSet, ColumnInfo, ColumnInput, ColumnValue, Editability, RowChange, TableStructure,
    TypeFamily,
};
use crate::value::{CellInput, Value};

/// One statement per change, in order. UPDATE/DELETE match on the primary key;
/// UPDATE also matches the original values of changed columns when they are
/// complete (`Value::is_complete`) so concurrent edits are detected (0 rows
/// affected → error "Row was changed or deleted by someone else").
pub fn change_statements(
    dialect: Dialect,
    structure: &TableStructure,
    changes: &ChangeSet,
) -> Result<Vec<String>> {
    if matches!(dialect, Dialect::Mongo) {
        return Err(Error::Unsupported(
            "SQL statements can't be built for MongoDB.".to_owned(),
        ));
    }
    if let Editability::ReadOnly(reason) = &structure.editability {
        return Err(Error::InvalidInput(reason.clone()));
    }
    let table = dialect.qualified(&changes.table);
    changes
        .changes
        .iter()
        .map(|change| match change {
            RowChange::Insert { values } => insert(dialect, structure, &table, values),
            RowChange::Update { key, original, set } => {
                update(dialect, structure, &table, key, original, set)
            }
            RowChange::Delete { key } => {
                let wheres = key_conditions(dialect, structure, key)?;
                Ok(format!(
                    "DELETE FROM {table} WHERE {}",
                    wheres.join(" AND ")
                ))
            }
        })
        .collect()
}

fn column<'a>(structure: &'a TableStructure, name: &str) -> Result<&'a ColumnInfo> {
    structure
        .columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "The table \"{}\" has no column \"{name}\".",
                structure.table.name
            ))
        })
}

fn insert(
    dialect: Dialect,
    structure: &TableStructure,
    table: &str,
    values: &[ColumnInput],
) -> Result<String> {
    let mut cols = Vec::with_capacity(values.len());
    let mut vals = Vec::with_capacity(values.len());
    for v in values {
        column(structure, &v.column)?;
        // SQLite has no DEFAULT keyword in VALUES; leaving the column out has the same effect.
        if matches!(dialect, Dialect::Sqlite) && v.value == CellInput::Default {
            continue;
        }
        cols.push(dialect.quote_ident(&v.column));
        vals.push(dialect.input_literal(&v.value)?);
    }
    Ok(if cols.is_empty() {
        match dialect {
            Dialect::Mysql { .. } => format!("INSERT INTO {table} () VALUES ()"),
            _ => format!("INSERT INTO {table} DEFAULT VALUES"),
        }
    } else {
        format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            cols.join(", "),
            vals.join(", ")
        )
    })
}

fn update(
    dialect: Dialect,
    structure: &TableStructure,
    table: &str,
    key: &[ColumnValue],
    original: &[ColumnValue],
    set: &[ColumnInput],
) -> Result<String> {
    if set.is_empty() {
        return Err(Error::InvalidInput(
            "An update needs at least one changed column.".to_owned(),
        ));
    }
    let mut assignments = Vec::with_capacity(set.len());
    for s in set {
        column(structure, &s.column)?;
        if matches!(dialect, Dialect::Sqlite) && s.value == CellInput::Default {
            return Err(Error::InvalidInput(
                "SQLite can't reset a column to its default; enter the value instead.".to_owned(),
            ));
        }
        assignments.push(format!(
            "{} = {}",
            dialect.quote_ident(&s.column),
            dialect.input_literal(&s.value)?
        ));
    }

    let mut wheres = key_conditions(dialect, structure, key)?;
    for o in original {
        if key.iter().any(|k| k.column == o.column) {
            continue;
        }
        let col = column(structure, &o.column)?;
        let ident = dialect.quote_ident(&o.column);
        if o.value.is_null() {
            wheres.push(format!("{ident} IS NULL"));
        } else if comparable(dialect, col, &o.value) {
            wheres.push(format!("{ident} = {}", dialect.literal(&o.value)?));
        }
    }
    Ok(format!(
        "UPDATE {table} SET {} WHERE {}",
        assignments.join(", "),
        wheres.join(" AND ")
    ))
}

/// `pk = value` conditions; every primary key column must be present.
fn key_conditions(
    dialect: Dialect,
    structure: &TableStructure,
    key: &[ColumnValue],
) -> Result<Vec<String>> {
    if key.is_empty() {
        return Err(Error::InvalidInput(
            "The row has no key values, so it can't be identified.".to_owned(),
        ));
    }
    for pk in &structure.primary_key {
        if !key.iter().any(|k| &k.column == pk) {
            return Err(Error::InvalidInput(format!(
                "The key is missing the primary key column \"{pk}\"."
            )));
        }
    }
    key.iter()
        .map(|k| {
            column(structure, &k.column)?;
            let ident = dialect.quote_ident(&k.column);
            Ok(if k.value.is_null() {
                format!("{ident} IS NULL")
            } else {
                format!("{ident} = {}", dialect.literal(&k.value)?)
            })
        })
        .collect()
}

/// Whether comparing the column to the loaded value with `=` is reliable. Values
/// the server rendered lossily or types without a usable equality operator are
/// left out of the optimistic-concurrency check (the primary key still applies).
fn comparable(dialect: Dialect, col: &ColumnInfo, value: &Value) -> bool {
    if !value.is_complete() {
        return false;
    }
    let ty = col.data_type.to_ascii_lowercase();
    match dialect {
        Dialect::Postgres => match col.family {
            // real (float4) widens to double and no longer equals the shortest decimal text.
            TypeFamily::Float => !(ty.starts_with("real") || ty.starts_with("float4")),
            // json has no equality operator; jsonb does.
            TypeFamily::Json => ty == "jsonb",
            TypeFamily::Array => {
                !((ty.contains("json") && !ty.contains("jsonb")) || ty.contains("xml"))
            }
            TypeFamily::Spatial | TypeFamily::Other => false,
            _ => true,
        },
        Dialect::Mysql { .. } => match col.family {
            TypeFamily::Float => !ty.starts_with("float"),
            // A JSON column compared with a string literal compares as a JSON string.
            TypeFamily::Json | TypeFamily::Spatial => false,
            _ => true,
        },
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TableKind, TableRef};
    use serde_json::json;

    fn col(name: &str, ty: &str, family: TypeFamily, pk: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: ty.into(),
            family,
            nullable: !pk,
            default: None,
            is_primary_key: pk,
            is_auto_increment: false,
            is_generated: false,
            enum_values: None,
            comment: None,
        }
    }

    fn table() -> TableRef {
        TableRef {
            database: "app".into(),
            schema: Some("public".into()),
            name: "users".into(),
        }
    }

    fn structure() -> TableStructure {
        TableStructure {
            table: table(),
            kind: TableKind::Table,
            columns: vec![
                col("id", "integer", TypeFamily::Integer, true),
                col("name", "text", TypeFamily::Text, false),
                col("score", "real", TypeFamily::Float, false),
                col("data", "json", TypeFamily::Json, false),
                col("meta", "jsonb", TypeFamily::Json, false),
                col("blob", "bytea", TypeFamily::Binary, false),
                col("select", "text", TypeFamily::Text, false),
            ],
            primary_key: vec!["id".into()],
            indexes: vec![],
            foreign_keys: vec![],
            editability: Editability::Editable,
            ddl: None,
        }
    }

    fn set(changes: Vec<RowChange>) -> ChangeSet {
        ChangeSet {
            table: table(),
            changes,
        }
    }

    fn text(c: &str, v: &str) -> ColumnInput {
        ColumnInput {
            column: c.into(),
            value: CellInput::Text(v.into()),
        }
    }

    fn key(id: i64) -> Vec<ColumnValue> {
        vec![ColumnValue {
            column: "id".into(),
            value: Value::Int(id),
        }]
    }

    fn orig(c: &str, v: Value) -> ColumnValue {
        ColumnValue {
            column: c.into(),
            value: v,
        }
    }

    const MY: Dialect = Dialect::Mysql {
        backslash_escapes: true,
    };

    #[test]
    fn postgres_statements() {
        let cs = set(vec![
            RowChange::Insert {
                values: vec![
                    text("name", "O'Brien"),
                    ColumnInput {
                        column: "id".into(),
                        value: CellInput::Default,
                    },
                    ColumnInput {
                        column: "select".into(),
                        value: CellInput::Null,
                    },
                ],
            },
            RowChange::Update {
                key: key(7),
                original: vec![orig("name", Value::Text("old".into()))],
                set: vec![text("name", "new")],
            },
            RowChange::Delete { key: key(7) },
        ]);
        let sql = change_statements(Dialect::Postgres, &structure(), &cs).unwrap();
        assert_eq!(
            sql,
            [
                "INSERT INTO \"public\".\"users\" (\"name\", \"id\", \"select\") VALUES ('O''Brien', DEFAULT, NULL)",
                "UPDATE \"public\".\"users\" SET \"name\" = 'new' WHERE \"id\" = 7 AND \"name\" = 'old'",
                "DELETE FROM \"public\".\"users\" WHERE \"id\" = 7",
            ]
        );
    }

    #[test]
    fn mysql_statements() {
        let mut t = table();
        t.schema = None;
        let cs = ChangeSet {
            table: t,
            changes: vec![
                RowChange::Insert {
                    values: vec![text("name", "a\\b")],
                },
                RowChange::Insert { values: vec![] },
                RowChange::Update {
                    key: key(1),
                    original: vec![orig("name", Value::Null)],
                    set: vec![text("name", "x")],
                },
            ],
        };
        let sql = change_statements(MY, &structure(), &cs).unwrap();
        assert_eq!(
            sql,
            [
                "INSERT INTO `app`.`users` (`name`) VALUES ('a\\\\b')",
                "INSERT INTO `app`.`users` () VALUES ()",
                "UPDATE `app`.`users` SET `name` = 'x' WHERE `id` = 1 AND `name` IS NULL",
            ]
        );
    }

    #[test]
    fn sqlite_omits_default_columns() {
        let mut t = table();
        t.database = "main".into();
        let cs = ChangeSet {
            table: t,
            changes: vec![
                RowChange::Insert {
                    values: vec![
                        ColumnInput {
                            column: "id".into(),
                            value: CellInput::Default,
                        },
                        text("name", "n"),
                    ],
                },
                RowChange::Insert {
                    values: vec![ColumnInput {
                        column: "id".into(),
                        value: CellInput::Default,
                    }],
                },
            ],
        };
        let sql = change_statements(Dialect::Sqlite, &structure(), &cs).unwrap();
        assert_eq!(
            sql,
            [
                "INSERT INTO \"users\" (\"name\") VALUES ('n')",
                "INSERT INTO \"users\" DEFAULT VALUES",
            ]
        );
        let bad = set(vec![RowChange::Update {
            key: key(1),
            original: vec![],
            set: vec![ColumnInput {
                column: "name".into(),
                value: CellInput::Default,
            }],
        }]);
        assert!(change_statements(Dialect::Sqlite, &structure(), &bad).is_err());
    }

    #[test]
    fn originals_are_skipped_when_unreliable() {
        let cs = set(vec![RowChange::Update {
            key: key(1),
            original: vec![
                // Truncated text and partial bytes can't be compared.
                orig(
                    "name",
                    Value::TruncatedText {
                        prefix: "abc".into(),
                        len: 99_999,
                    },
                ),
                orig(
                    "blob",
                    Value::Bytes {
                        data: vec![1],
                        len: 9_000,
                    },
                ),
                // real and json have no reliable equality in Postgres.
                orig("score", Value::Float(0.1)),
                orig("data", Value::Json(json!({"a": 1}))),
                // jsonb does.
                orig("meta", Value::Json(json!({"a": 1}))),
            ],
            set: vec![text("name", "x")],
        }]);
        let sql = change_statements(Dialect::Postgres, &structure(), &cs).unwrap();
        assert_eq!(
            sql,
            [
                "UPDATE \"public\".\"users\" SET \"name\" = 'x' WHERE \"id\" = 1 AND \"meta\" = '{\"a\":1}'"
            ]
        );
    }

    #[test]
    fn composite_and_null_keys() {
        let mut s = structure();
        s.primary_key = vec!["id".into(), "name".into()];
        let cs = set(vec![RowChange::Delete {
            key: vec![
                orig("id", Value::Int(1)),
                orig("name", Value::Text("a".into())),
            ],
        }]);
        let sql = change_statements(Dialect::Sqlite, &s, &cs).unwrap();
        assert_eq!(
            sql,
            ["DELETE FROM \"app\".\"users\" WHERE \"id\" = 1 AND \"name\" = 'a'"]
        );
        // Missing a key column is refused.
        let partial = set(vec![RowChange::Delete { key: key(1) }]);
        assert!(change_statements(Dialect::Postgres, &s, &partial).is_err());
        // A NULL key part renders IS NULL.
        let nulls = set(vec![RowChange::Delete {
            key: vec![
                orig("id", Value::Null),
                orig("name", Value::Text("a".into())),
            ],
        }]);
        assert!(
            change_statements(Dialect::Postgres, &s, &nulls).unwrap()[0].contains("\"id\" IS NULL")
        );
    }

    #[test]
    fn refuses_bad_input() {
        let mut ro = structure();
        ro.editability = Editability::ReadOnly("Views can't be edited.".into());
        let cs = set(vec![RowChange::Delete { key: key(1) }]);
        assert!(matches!(
            change_statements(Dialect::Postgres, &ro, &cs),
            Err(Error::InvalidInput(m)) if m == "Views can't be edited."
        ));

        let unknown = set(vec![RowChange::Insert {
            values: vec![text("nope", "x")],
        }]);
        assert!(change_statements(Dialect::Postgres, &structure(), &unknown).is_err());

        let empty_set = set(vec![RowChange::Update {
            key: key(1),
            original: vec![],
            set: vec![],
        }]);
        assert!(change_statements(Dialect::Postgres, &structure(), &empty_set).is_err());

        let no_key = set(vec![RowChange::Delete { key: vec![] }]);
        assert!(change_statements(Dialect::Postgres, &structure(), &no_key).is_err());

        assert!(matches!(
            change_statements(Dialect::Mongo, &structure(), &cs),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn binary_and_json_inputs() {
        let cs = set(vec![RowChange::Update {
            key: key(1),
            original: vec![],
            set: vec![
                ColumnInput {
                    column: "blob".into(),
                    value: CellInput::Bytes("3q0=".into()),
                },
                ColumnInput {
                    column: "data".into(),
                    value: CellInput::Json(json!({"k": [1, 2]})),
                },
            ],
        }]);
        let pg = change_statements(Dialect::Postgres, &structure(), &cs).unwrap();
        assert_eq!(
            pg[0],
            "UPDATE \"public\".\"users\" SET \"blob\" = '\\xdead'::bytea, \"data\" = '{\"k\":[1,2]}' WHERE \"id\" = 1"
        );
        let my = change_statements(MY, &structure(), &cs).unwrap();
        assert!(my[0].contains("`blob` = X'dead'"));
    }
}
