//! `ChannelCursor`: the standard `Cursor` implementation. A driver spawns a task
//! that owns a connection, runs the statement and sends `CursorEvent`s through a
//! bounded channel; the channel's capacity is the backpressure that keeps memory
//! bounded while the UI is not asking for more rows.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::driver::{CancelHandle, Cursor, CursorEvent};
use crate::error::{Error, Result};
use crate::value::Row;

/// Batches the producer should send; the consumer re-chunks to whatever `max_rows` it asks for.
pub const PRODUCER_BATCH: usize = 256;
/// Channel capacity in events: at most this many batches are buffered ahead of the consumer.
pub const CHANNEL_CAPACITY: usize = 8;

/// Producer side handed to the driver's task.
pub type EventSender = mpsc::Sender<Result<CursorEvent>>;

pub struct ChannelCursor {
    rx: mpsc::Receiver<Result<CursorEvent>>,
    leftover: VecDeque<Row>,
    cancel: Arc<dyn CancelHandle>,
    task: Option<JoinHandle<()>>,
    /// Dropping the cursor lets the producer finish the statement instead of aborting it.
    detach_on_drop: bool,
}

impl ChannelCursor {
    /// Spawns `produce` with the sender; it should send events and return when done.
    /// Errors returned by `produce` are delivered to the consumer.
    pub fn spawn<F, Fut>(cancel: Arc<dyn CancelHandle>, produce: F) -> Self
    where
        F: FnOnce(EventSender) -> Fut,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let fut = produce(tx.clone());
        let task = tokio::spawn(async move {
            if let Err(err) = fut.await {
                let _ = tx.send(Err(err)).await;
            }
        });
        ChannelCursor {
            rx,
            leftover: VecDeque::new(),
            cancel,
            task: Some(task),
            detach_on_drop: false,
        }
    }

    /// For producers that must not be aborted (they own the one connection of a manual
    /// transaction): when the cursor is dropped the task keeps running, reads the statement to
    /// its end without sending anything, and gives the connection back.
    pub fn detach_on_drop(mut self) -> Self {
        self.detach_on_drop = true;
        self
    }
}

#[async_trait]
impl Cursor for ChannelCursor {
    async fn next(&mut self, max_rows: usize) -> Result<Option<CursorEvent>> {
        let max_rows = max_rows.max(1);
        if !self.leftover.is_empty() {
            let n = max_rows.min(self.leftover.len());
            return Ok(Some(CursorEvent::Rows(self.leftover.drain(..n).collect())));
        }
        match self.rx.recv().await {
            None => Ok(None),
            Some(Err(e)) => Err(e),
            Some(Ok(CursorEvent::Rows(mut rows))) if rows.len() > max_rows => {
                self.leftover.extend(rows.drain(max_rows..));
                Ok(Some(CursorEvent::Rows(rows)))
            }
            Some(Ok(event)) => Ok(Some(event)),
        }
    }

    fn cancel_handle(&self) -> Arc<dyn CancelHandle> {
        self.cancel.clone()
    }
}

impl Drop for ChannelCursor {
    fn drop(&mut self) {
        if let Some(task) = self.task.take()
            && !self.detach_on_drop
        {
            task.abort();
        }
    }
}

/// Cancel handle for engines where dropping the producer is enough.
pub struct NoopCancel;

#[async_trait]
impl CancelHandle for NoopCancel {
    async fn cancel(&self) -> Result<()> {
        Ok(())
    }
}

/// A cursor over rows already in memory (small metadata results, tests).
pub struct VecCursor {
    events: VecDeque<CursorEvent>,
}

impl VecCursor {
    pub fn new(events: Vec<CursorEvent>) -> Self {
        VecCursor {
            events: events.into(),
        }
    }
}

#[async_trait]
impl Cursor for VecCursor {
    async fn next(&mut self, max_rows: usize) -> Result<Option<CursorEvent>> {
        match self.events.pop_front() {
            Some(CursorEvent::Rows(mut rows)) if rows.len() > max_rows.max(1) => {
                let rest = rows.split_off(max_rows.max(1));
                self.events.push_front(CursorEvent::Rows(rest));
                Ok(Some(CursorEvent::Rows(rows)))
            }
            other => Ok(other),
        }
    }

    fn cancel_handle(&self) -> Arc<dyn CancelHandle> {
        Arc::new(NoopCancel)
    }
}

/// Maps a closed channel (consumer gone) to `Cancelled`, for use with `?` in producers.
pub async fn send(tx: &EventSender, event: CursorEvent) -> Result<()> {
    tx.send(Ok(event)).await.map_err(|_| Error::Cancelled)
}
