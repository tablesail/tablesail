//! Application state shared by all commands: open sessions, running queries,
//! background jobs, and paths for config and caches.

/// A3 owns the fields: sessions (id → dbcore::Session), queries (id → buffered
/// statement results with cursors), jobs (id → progress + CancellationToken),
/// config dir (connections.json) and cache dir (schema snapshots).
#[derive(Default)]
pub struct AppState {}
