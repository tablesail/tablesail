//! Saved connections: `connections.json` in the app config directory.
//! Secrets never touch this file (see `keychain`).

use std::path::{Path, PathBuf};

use dbcore::Error;
use dbcore::model::ConnectionConfig;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

type Result<T> = std::result::Result<T, Error>;

const FILE: &str = "connections.json";

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    connections: Vec<ConnectionConfig>,
}

fn file_path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

pub async fn load(config_dir: &Path) -> Result<Vec<ConnectionConfig>> {
    let path = file_path(config_dir);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(format!("cannot read {}: {e}", path.display()))),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    parse(&bytes).map_err(|e| {
        Error::Other(format!(
            "{} is not valid ({e}). Fix or delete the file to continue.",
            path.display()
        ))
    })
}

fn parse(bytes: &[u8]) -> std::result::Result<Vec<ConnectionConfig>, serde_json::Error> {
    // Accept a bare array too, so hand-written files work.
    match serde_json::from_slice::<Document>(bytes) {
        Ok(doc) => Ok(doc.connections),
        Err(first) => serde_json::from_slice::<Vec<ConnectionConfig>>(bytes).map_err(|_| first),
    }
}

/// Writes the whole list through a temp file + rename, so a crash never leaves a half-written file.
pub async fn save(config_dir: &Path, connections: &[ConnectionConfig]) -> Result<()> {
    tokio::fs::create_dir_all(config_dir)
        .await
        .map_err(|e| Error::Io(format!("cannot create {}: {e}", config_dir.display())))?;
    let doc = Document {
        version: 1,
        connections: connections.to_vec(),
    };
    let json = serde_json::to_vec_pretty(&doc).map_err(|e| Error::Other(e.to_string()))?;
    let path = file_path(config_dir);
    let tmp = config_dir.join(format!("{FILE}.tmp"));
    let write = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(&json).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, &path).await
    };
    write
        .await
        .map_err(|e| Error::Io(format!("cannot write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcore::Engine;

    fn sample(id: &str) -> ConnectionConfig {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": "local", "engine": "postgres", "host": "localhost", "port": 5432
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn round_trips_and_replaces_atomically() {
        let dir = std::env::temp_dir().join(format!("tablesail-store-{}", uuid::Uuid::new_v4()));
        assert!(load(&dir).await.unwrap().is_empty());
        save(&dir, &[sample("a"), sample("b")]).await.unwrap();
        save(&dir, &[sample("a")]).await.unwrap();
        let list = load(&dir).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "a");
        assert_eq!(list[0].engine, Engine::Postgres);
        assert!(!dir.join("connections.json.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn corrupt_file_is_reported_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("tablesail-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("connections.json"), "{ nope").unwrap();
        let err = load(&dir).await.unwrap_err();
        assert!(err.to_string().contains("connections.json"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn accepts_a_bare_array() {
        let json = serde_json::to_vec(&vec![sample("x")]).unwrap();
        assert_eq!(parse(&json).unwrap().len(), 1);
    }
}
