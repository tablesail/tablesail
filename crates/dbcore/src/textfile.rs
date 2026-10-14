//! Plain UTF-8 text files for the SQL editor (open / save a .sql file).

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Error, Result};

/// Largest file the editor opens. Bigger dumps go through `sqlimport`, which streams.
pub const MAX_READ_BYTES: u64 = 50 * 1024 * 1024;

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn io(action: &str, path: &Path, err: std::io::Error) -> Error {
    Error::Io(format!("cannot {action} {}: {err}", path.display()))
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Reads a UTF-8 text file. A leading byte order mark is dropped, so it doesn't end up as an
/// invisible character in front of the first statement.
///
/// Files over [`MAX_READ_BYTES`] and files that are not valid UTF-8 are `InvalidInput`.
pub async fn read_text(path: &str) -> Result<String> {
    if path.trim().is_empty() {
        return Err(Error::InvalidInput("Choose a file to open.".into()));
    }
    let path = Path::new(path);
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| io("read", path, e))?;
    let meta = file.metadata().await.map_err(|e| io("read", path, e))?;
    if !meta.is_file() {
        return Err(Error::InvalidInput(format!(
            "{} is not a file.",
            path.display()
        )));
    }
    let too_big = |size: u64| {
        Error::InvalidInput(format!(
            "{} is {}, which is more than the {} the editor opens. Use Import SQL File to run a large dump.",
            path.display(),
            megabytes(size),
            megabytes(MAX_READ_BYTES),
        ))
    };
    if meta.len() > MAX_READ_BYTES {
        return Err(too_big(meta.len()));
    }
    // The file may grow while it is read, so the limit is enforced on the bytes too.
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| io("read", path, e))?;
    if bytes.len() as u64 > MAX_READ_BYTES {
        return Err(too_big(bytes.len() as u64));
    }
    decode(path, bytes)
}

fn decode(path: &Path, mut bytes: Vec<u8>) -> Result<String> {
    if bytes.starts_with(BOM) {
        bytes.drain(..BOM.len());
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(Error::InvalidInput(format!(
            "{} is UTF-16 text. Save it as UTF-8 and open it again.",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|e| {
        Error::InvalidInput(format!(
            "{} is not a UTF-8 text file (invalid byte at position {}).",
            path.display(),
            e.utf8_error().valid_up_to()
        ))
    })
}

/// Writes `contents` to `path` atomically: the text goes to a temporary file in the same
/// directory, which then replaces the target, so a crash or a full disk never leaves a
/// half-written file behind. An existing file keeps its permissions; a symbolic link is
/// written through.
pub async fn write_text(path: &str, contents: &str) -> Result<()> {
    if path.trim().is_empty() {
        return Err(Error::InvalidInput("Choose a file to save to.".into()));
    }
    let requested = Path::new(path);
    // Replacing a symbolic link would turn it into a plain file; write to what it points at.
    let target: PathBuf = match tokio::fs::canonicalize(requested).await {
        Ok(real) => real,
        Err(_) => requested.to_path_buf(),
    };
    if tokio::fs::metadata(&target)
        .await
        .is_ok_and(|meta| meta.is_dir())
    {
        return Err(Error::InvalidInput(format!(
            "{} is a folder, not a file.",
            requested.display()
        )));
    }
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.{}.tmp",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));

    let result: Result<()> = async {
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| io("write", requested, e))?;
        file.write_all(contents.as_bytes())
            .await
            .map_err(|e| io("write", requested, e))?;
        file.flush().await.map_err(|e| io("write", requested, e))?;
        // Durable before the rename makes it visible.
        file.sync_all()
            .await
            .map_err(|e| io("write", requested, e))?;
        drop(file);
        if let Ok(meta) = tokio::fs::metadata(&target).await {
            let _ = tokio::fs::set_permissions(&tmp, meta.permissions()).await;
        }
        tokio::fs::rename(&tmp, &target)
            .await
            .map_err(|e| io("write", requested, e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tablesail-textfile-{label}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn path_str(p: &Path) -> &str {
        p.to_str().unwrap()
    }

    #[tokio::test]
    async fn writes_and_reads_back_text() {
        let dir = scratch_dir("roundtrip");
        let file = dir.join("q.sql");
        let text = "SELECT 'héllo wörld ✓';\n-- 日本語\r\nSELECT 2;";
        write_text(path_str(&file), text).await.unwrap();
        assert_eq!(read_text(path_str(&file)).await.unwrap(), text);

        // Overwriting replaces the contents and leaves no temporary file behind.
        write_text(path_str(&file), "SELECT 3;").await.unwrap();
        assert_eq!(read_text(path_str(&file)).await.unwrap(), "SELECT 3;");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["q.sql"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_empty_file_reads_as_empty_text() {
        let dir = scratch_dir("empty");
        let file = dir.join("empty.sql");
        write_text(path_str(&file), "").await.unwrap();
        assert_eq!(read_text(path_str(&file)).await.unwrap(), "");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_byte_order_mark_is_dropped() {
        let dir = scratch_dir("bom");
        let file = dir.join("bom.sql");
        std::fs::write(&file, b"\xEF\xBB\xBFSELECT 1;").unwrap();
        assert_eq!(read_text(path_str(&file)).await.unwrap(), "SELECT 1;");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn invalid_utf8_and_utf16_are_clear_errors() {
        let dir = scratch_dir("encoding");
        let latin1 = dir.join("latin1.sql");
        std::fs::write(&latin1, b"SELECT 'caf\xE9';").unwrap();
        let err = read_text(path_str(&latin1)).await.unwrap_err();
        match err {
            Error::InvalidInput(m) => {
                assert!(m.contains("not a UTF-8 text file"), "{m}");
                assert!(m.contains("position 11"), "{m}");
            }
            other => panic!("{other:?}"),
        }

        let utf16 = dir.join("utf16.sql");
        std::fs::write(&utf16, b"\xFF\xFES\0E\0").unwrap();
        match read_text(path_str(&utf16)).await.unwrap_err() {
            Error::InvalidInput(m) => assert!(m.contains("UTF-16"), "{m}"),
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn files_over_the_limit_are_refused_without_reading_them() {
        let dir = scratch_dir("big");
        let file = dir.join("big.sql");
        let handle = std::fs::File::create(&file).unwrap();
        handle.set_len(MAX_READ_BYTES + 1).unwrap(); // sparse: costs no disk
        drop(handle);
        match read_text(path_str(&file)).await.unwrap_err() {
            Error::InvalidInput(m) => {
                assert!(m.contains("50.0 MB"), "{m}");
                assert!(m.contains("Import SQL File"), "{m}");
            }
            other => panic!("{other:?}"),
        }
        // Exactly at the limit is fine.
        let handle = std::fs::File::create(&file).unwrap();
        handle.set_len(MAX_READ_BYTES).unwrap();
        drop(handle);
        let text = read_text(path_str(&file)).await.unwrap();
        assert_eq!(text.len() as u64, MAX_READ_BYTES);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn io_problems_are_io_errors() {
        let dir = scratch_dir("io");
        let missing = dir.join("nope.sql");
        match read_text(path_str(&missing)).await.unwrap_err() {
            Error::Io(m) => assert!(m.contains("nope.sql"), "{m}"),
            other => panic!("{other:?}"),
        }
        // The folder does not exist.
        let orphan = dir.join("no-such-dir").join("a.sql");
        assert!(matches!(
            write_text(path_str(&orphan), "x").await.unwrap_err(),
            Error::Io(_)
        ));
        // A folder is neither readable nor writable as text.
        assert!(matches!(
            read_text(path_str(&dir)).await.unwrap_err(),
            Error::InvalidInput(_) | Error::Io(_)
        ));
        assert!(matches!(
            write_text(path_str(&dir), "x").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            read_text("  ").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
        assert!(matches!(
            write_text("", "x").await.unwrap_err(),
            Error::InvalidInput(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn saving_keeps_permissions_and_symbolic_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = scratch_dir("unix");
        let real = dir.join("real.sql");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("link.sql");
        symlink(&real, &link).unwrap();

        write_text(path_str(&link), "new").await.unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link is still a link"
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
