//! Streaming export (cursor → file, never the whole result in memory) and CSV import.

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::driver::Driver;
use crate::error::Result;
use crate::model::{CsvImport, CsvPreview, ExportFormat, ExportSource, JobProgress};

/// Called with progress at most every ~100 ms and once at the end.
pub type ProgressFn = Arc<dyn Fn(&JobProgress) + Send + Sync>;

/// Streams `source` to `path` in `format`. Cancelling leaves a partial file and
/// returns progress with `cancelled: true`.
pub async fn export(
    driver: Arc<dyn Driver>,
    source: ExportSource,
    format: ExportFormat,
    path: PathBuf,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    todo!(
        "A3: export {source:?} as {format:?} to {path:?} ({:?}, {})",
        driver.engine(),
        cancel.is_cancelled() as u8 + progress_dummy(&progress)
    )
}

fn progress_dummy(_p: &ProgressFn) -> u8 {
    0
}

/// Reads the first rows of a CSV file; `delimiter` None means sniff (`,` `;` tab `|`).
pub fn csv_preview(path: &str, delimiter: Option<char>, rows: usize) -> Result<CsvPreview> {
    todo!("A3: preview {path} {delimiter:?} {rows}")
}

/// Imports a CSV file in batches through `Driver::insert_rows`, in one transaction per batch.
pub async fn import_csv(
    driver: Arc<dyn Driver>,
    import: CsvImport,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<JobProgress> {
    todo!(
        "A3: import {import:?} ({:?} {})",
        driver.engine(),
        cancel.is_cancelled() as u8 + progress_dummy(&progress)
    )
}
