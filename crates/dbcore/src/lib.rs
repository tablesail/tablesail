//! Database core for TableSoft.
//!
//! Everything that talks to a database lives here, independent of Tauri, so it
//! can be built and tested without the UI. Phase 1 adds the `Driver` trait,
//! the typed `Value` enum and the SQLite adapter.

/// Version of the dbcore crate, shown in the UI to prove the Rust bridge works.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Engines supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
}
