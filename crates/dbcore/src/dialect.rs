//! Per-engine SQL syntax: quoting, literals, LIMIT/OFFSET, filter rendering.

use serde::{Deserialize, Serialize};

use crate::Engine;
use crate::error::Result;
use crate::model::{Filter, TableRef};
use crate::value::{CellInput, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Dialect {
    Sqlite,
    Postgres,
    /// MySQL and MariaDB. `backslash_escapes` is false when sql_mode has NO_BACKSLASH_ESCAPES.
    Mysql {
        backslash_escapes: bool,
    },
    /// Mongo shell syntax (used by the splitter only).
    Mongo,
}

impl Dialect {
    pub fn for_engine(engine: Engine) -> Self {
        match engine {
            Engine::Sqlite => Dialect::Sqlite,
            Engine::Postgres => Dialect::Postgres,
            Engine::Mysql | Engine::Mariadb => Dialect::Mysql {
                backslash_escapes: true,
            },
            Engine::Mongodb => Dialect::Mongo,
        }
    }

    /// Quotes an identifier: `"a""b"` (Postgres/SQLite) or `` `a``b` `` (MySQL).
    pub fn quote_ident(&self, ident: &str) -> String {
        match self {
            Dialect::Mysql { .. } => format!("`{}`", ident.replace('`', "``")),
            _ => format!("\"{}\"", ident.replace('"', "\"\"")),
        }
    }

    /// Fully qualified, quoted table name (`"schema"."table"`, `` `db`.`table` ``,
    /// SQLite: `"db"."table"` unless db is "main").
    pub fn qualified(&self, table: &TableRef) -> String {
        match self {
            Dialect::Postgres => match &table.schema {
                Some(s) => format!("{}.{}", self.quote_ident(s), self.quote_ident(&table.name)),
                None => self.quote_ident(&table.name),
            },
            Dialect::Mysql { .. } => {
                if table.database.is_empty() {
                    self.quote_ident(&table.name)
                } else {
                    format!(
                        "{}.{}",
                        self.quote_ident(&table.database),
                        self.quote_ident(&table.name)
                    )
                }
            }
            Dialect::Sqlite => {
                if table.database.is_empty() || table.database == "main" {
                    self.quote_ident(&table.name)
                } else {
                    format!(
                        "{}.{}",
                        self.quote_ident(&table.database),
                        self.quote_ident(&table.name)
                    )
                }
            }
            Dialect::Mongo => table.name.clone(),
        }
    }

    /// SQL string literal with dialect-correct escaping.
    pub fn quote_string(&self, s: &str) -> String {
        todo!("A1: dialect string literal for {s:?}")
    }

    /// SQL literal for a loaded value (used in WHERE clauses of change sets and SQL export).
    pub fn literal(&self, value: &Value) -> Result<String> {
        todo!("A1: literal for {value:?}")
    }

    /// SQL literal for user input (`CellInput::Default` renders `DEFAULT`).
    pub fn input_literal(&self, input: &CellInput) -> Result<String> {
        todo!("A1: input literal for {input:?}")
    }

    /// `LIMIT n OFFSET m` (all three SQL dialects accept this form).
    pub fn limit_offset(&self, limit: u32, offset: u64) -> String {
        format!("LIMIT {limit} OFFSET {offset}")
    }

    /// Renders one filter as a boolean SQL expression.
    pub fn filter_sql(&self, filter: &Filter) -> Result<String> {
        todo!("A1: filter SQL for {filter:?}")
    }
}
