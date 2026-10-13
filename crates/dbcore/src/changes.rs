//! Turns a `ChangeSet` into SQL statements (Postgres, MySQL, SQLite).

use crate::dialect::Dialect;
use crate::error::Result;
use crate::model::{ChangeSet, TableStructure};

/// One statement per change, in order. UPDATE/DELETE match on the primary key;
/// UPDATE also matches the original values of changed columns when they are
/// complete (`Value::is_complete`) so concurrent edits are detected (0 rows
/// affected → error "Row was changed or deleted by someone else").
pub fn change_statements(
    dialect: Dialect,
    structure: &TableStructure,
    changes: &ChangeSet,
) -> Result<Vec<String>> {
    todo!(
        "A1: build {} change statements for {dialect:?} on {}",
        changes.changes.len(),
        structure.table.name
    )
}
