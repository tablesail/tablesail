//! Database core for TableSoft.
//!
//! Everything that talks to a database lives here, independent of Tauri, so it
//! can be built and tested without the UI.

pub mod changes;
pub mod cursor;
pub mod dialect;
pub mod driver;
pub mod drivers;
pub mod error;
pub mod export;
pub mod model;
pub mod session;
pub mod split;
pub mod ssh;
pub mod value;

pub use dialect::Dialect;
pub use driver::{CancelHandle, Cursor, CursorEvent, Driver};
pub use error::{Error, Result};
pub use session::Session;
pub use value::{CellInput, Row, Value};

/// Version of the dbcore crate, shown in the UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Engines supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Sqlite,
    Postgres,
    Mysql,
    Mariadb,
    Mongodb,
}

impl Engine {
    pub const ALL: [Engine; 5] = [
        Engine::Sqlite,
        Engine::Postgres,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Mongodb,
    ];

    pub fn default_port(self) -> u16 {
        match self {
            Engine::Sqlite => 0,
            Engine::Postgres => 5432,
            Engine::Mysql | Engine::Mariadb => 3306,
            Engine::Mongodb => 27017,
        }
    }
}
