//! Schema snapshots: built from the server, cached on disk so the next launch is instant.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dbcore::Result;
use dbcore::driver::Driver;
use dbcore::model::{SchemaSnapshot, TableInfo, TableKind};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Column names are only collected below this many tables; beyond it the
/// introspection would take longer than the user is willing to wait.
const COLUMN_NAME_LIMIT: usize = 2000;
const CONCURRENCY: usize = 8;

/// File-name-safe form of a database name (unique: escapes use `%`, the empty name uses `~`).
fn file_stem(database: &str) -> String {
    if database.is_empty() {
        return "~default".to_owned();
    }
    let mut out = String::with_capacity(database.len());
    for b in database.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    // A name made only of dots would be a path component with special meaning.
    if out.chars().all(|c| c == '.') {
        out = out.replace('.', "%2E");
    }
    out
}

pub fn connection_dir(cache_dir: &Path, connection_id: &str) -> PathBuf {
    cache_dir.join("schema").join(file_stem(connection_id))
}

fn snapshot_path(cache_dir: &Path, connection_id: &str, database: &str) -> PathBuf {
    connection_dir(cache_dir, connection_id).join(format!("{}.json", file_stem(database)))
}

pub async fn read_cached(
    cache_dir: &Path,
    connection_id: &str,
    database: &str,
) -> Option<SchemaSnapshot> {
    let bytes = tokio::fs::read(snapshot_path(cache_dir, connection_id, database))
        .await
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_cache(cache_dir: &Path, connection_id: &str, database: &str, snap: &SchemaSnapshot) {
    let path = snapshot_path(cache_dir, connection_id, database);
    let result: std::io::Result<()> = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec(snap).map_err(std::io::Error::other)?;
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(&json).await?;
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&tmp, &path).await
    }
    .await;
    if let Err(e) = result {
        // A cache that cannot be written only costs the next launch some speed.
        eprintln!("[schema] cannot write cache {}: {e}", path.display());
    }
}

pub async fn remove_cache(cache_dir: &Path, connection_id: &str) {
    let _ = tokio::fs::remove_dir_all(connection_dir(cache_dir, connection_id)).await;
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn has_columns(kind: TableKind) -> bool {
    !matches!(kind, TableKind::Sequence | TableKind::System)
}

/// Reads tables of every schema and, for databases of sane size, column names.
pub async fn build(driver: &Arc<dyn Driver>, database: &str) -> Result<SchemaSnapshot> {
    let schema_names = driver.list_schemas(database).await?;
    let mut schemas: BTreeMap<String, Vec<TableInfo>> = BTreeMap::new();
    if schema_names.is_empty() {
        schemas.insert(String::new(), driver.list_tables(database, None).await?);
    } else {
        for schema in schema_names {
            let tables = driver.list_tables(database, Some(&schema)).await?;
            schemas.insert(schema, tables);
        }
    }

    let total: usize = schemas.values().map(Vec::len).sum();
    let mut columns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if total <= COLUMN_NAME_LIMIT {
        let permits = Arc::new(Semaphore::new(CONCURRENCY));
        let mut tasks = JoinSet::new();
        for info in schemas.values().flatten().filter(|t| has_columns(t.kind)) {
            let table = info.table.clone();
            let driver = driver.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await.ok()?;
                // One unreadable table (permissions, dropped meanwhile) must not fail the snapshot.
                let structure = driver.table_structure(&table).await.ok()?;
                let key = match &table.schema {
                    Some(schema) => format!("{schema}.{}", table.name),
                    None => table.name.clone(),
                };
                Some((key, structure.columns.into_iter().map(|c| c.name).collect()))
            });
        }
        while let Some(joined) = tasks.join_next().await {
            if let Ok(Some((key, names))) = joined {
                columns.insert(key, names);
            }
        }
    }

    Ok(SchemaSnapshot {
        database: database.to_owned(),
        schemas,
        columns,
        fetched_at: unix_ms(),
    })
}

/// Cached snapshot unless `refresh`; otherwise builds and caches a fresh one.
pub async fn snapshot(
    cache_dir: &Path,
    connection_id: &str,
    driver: &Arc<dyn Driver>,
    database: &str,
    refresh: bool,
) -> Result<SchemaSnapshot> {
    if !refresh && let Some(cached) = read_cached(cache_dir, connection_id, database).await {
        return Ok(cached);
    }
    let snap = build(driver, database).await?;
    write_cache(cache_dir, connection_id, database, &snap).await;
    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems_are_safe_and_distinct() {
        assert_eq!(file_stem("shop"), "shop");
        assert_eq!(file_stem(""), "~default");
        assert_eq!(file_stem("a/b"), "a%2Fb");
        assert_eq!(file_stem(".."), "%2E%2E");
        assert_ne!(file_stem("a b"), file_stem("a%20b"));
    }
}
