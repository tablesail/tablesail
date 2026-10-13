//! Runs a parsed mongosh statement and streams its result through a `ChannelCursor`.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Stream, TryStreamExt, stream};
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::Hint;
use mongodb::{Client, Collection, Database};
use tokio_util::sync::CancellationToken;

use super::StructureCache;
use super::convert::db_err;
use super::infer::columns_of;
use super::shell::{FindOp, Op};
use super::{merge_columns, rows_of};
use crate::cursor::{EventSender, PRODUCER_BATCH, send};
use crate::driver::CursorEvent;
use crate::error::{Error, Result};
use crate::model::ColumnMeta;

/// Everything one running statement needs.
pub(super) struct ExecCtx {
    pub client: Client,
    pub database: String,
    /// Tags the operations of this statement so `killOp` can find them.
    pub comment: String,
    pub token: CancellationToken,
    pub structures: Arc<StructureCache>,
}

impl ExecCtx {
    fn db(&self) -> Database {
        self.client.database(&self.database)
    }

    fn collection(&self, name: &str) -> Collection<Document> {
        self.db().collection(name)
    }

    fn tag(&self) -> Bson {
        Bson::String(self.comment.clone())
    }

    /// Awaits a driver operation, abandoning it when the statement is cancelled.
    async fn guard<T, F>(&self, op: F) -> Result<T>
    where
        F: IntoFuture<Output = std::result::Result<T, mongodb::error::Error>>,
    {
        tokio::select! {
            biased;
            _ = self.token.cancelled() => Err(Error::Cancelled),
            r = op.into_future() => r.map_err(db_err),
        }
    }
}

async fn next_doc<S>(stream: &mut S, token: &CancellationToken) -> Result<Option<Document>>
where
    S: Stream<Item = std::result::Result<Document, mongodb::error::Error>> + Unpin,
{
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(Error::Cancelled),
        r = stream.try_next() => r.map_err(db_err),
    }
}

/// Streams documents as one result set. Columns are `base` plus any keys of the
/// first batch; documents arriving later cannot add columns.
pub(super) async fn emit_docs<S>(
    mut docs: S,
    base: Vec<ColumnMeta>,
    tx: &EventSender,
    token: &CancellationToken,
    grid: bool,
) -> Result<()>
where
    S: Stream<Item = std::result::Result<Document, mongodb::error::Error>> + Unpin,
{
    let mut batch = Vec::with_capacity(PRODUCER_BATCH);
    while batch.len() < PRODUCER_BATCH {
        match next_doc(&mut docs, token).await? {
            Some(d) => batch.push(d),
            None => break,
        }
    }
    let columns = if base.is_empty() {
        columns_of(&batch)
    } else {
        merge_columns(&base, &batch)
    };
    send(tx, CursorEvent::Columns(columns.clone())).await?;
    loop {
        if batch.is_empty() {
            return Ok(());
        }
        send(tx, CursorEvent::Rows(rows_of(&batch, &columns, grid))).await?;
        batch.clear();
        while batch.len() < PRODUCER_BATCH {
            match next_doc(&mut docs, token).await? {
                Some(d) => batch.push(d),
                None => break,
            }
        }
    }
}

/// A small in-memory result.
async fn emit_vec(docs: Vec<Document>, ctx: &ExecCtx, tx: &EventSender) -> Result<()> {
    emit_docs(
        stream::iter(docs.into_iter().map(Ok)),
        Vec::new(),
        tx,
        &ctx.token,
        true,
    )
    .await
}

async fn emit_affected(tx: &EventSender, n: u64) -> Result<()> {
    send(tx, CursorEvent::Affected(n)).await
}

/// A command reply: the documents of `cursor.firstBatch` when it is a cursor
/// reply, else the reply itself as one row.
async fn emit_reply(reply: Document, ctx: &ExecCtx, tx: &EventSender) -> Result<()> {
    if let Ok(cursor) = reply.get_document("cursor")
        && let Ok(batch) = cursor.get_array("firstBatch")
    {
        let docs = batch
            .iter()
            .filter_map(|b| b.as_document().cloned())
            .collect();
        return emit_vec(docs, ctx, tx).await;
    }
    emit_vec(vec![reply], ctx, tx).await
}

fn hint_of(bson: Bson) -> Option<Hint> {
    match bson {
        Bson::String(name) => Some(Hint::Name(name)),
        Bson::Document(keys) => Some(Hint::Keys(keys)),
        _ => None,
    }
}

/// `a_1_b_-1`, the name mongosh gives an index without one.
fn index_name(keys: &Document) -> String {
    keys.iter()
        .map(|(k, v)| match v {
            Bson::String(s) => format!("{k}_{s}"),
            Bson::Int32(i) => format!("{k}_{i}"),
            Bson::Int64(i) => format!("{k}_{i}"),
            Bson::Double(f) => format!("{k}_{}", *f as i64),
            other => format!("{k}_{other}"),
        })
        .collect::<Vec<_>>()
        .join("_")
}

async fn run_find(f: FindOp, ctx: &ExecCtx, tx: &EventSender) -> Result<()> {
    let coll = ctx.collection(&f.collection);
    let mut find = coll
        .find(f.filter)
        .comment(ctx.tag())
        .batch_size(if f.one { 1 } else { 500 });
    if let Some(sort) = f.sort {
        find = find.sort(sort);
    }
    if let Some(projection) = f.projection {
        find = find.projection(projection);
    }
    if f.skip > 0 {
        find = find.skip(f.skip);
    }
    match (f.one, f.limit) {
        (true, _) => find = find.limit(1),
        (false, Some(l)) if l > 0 => find = find.limit(l),
        _ => {}
    }
    if let Some(hint) = f.hint.and_then(hint_of) {
        find = find.hint(hint);
    }
    if let Some(ms) = f.max_time_ms.filter(|ms| *ms > 0) {
        find = find.max_time(Duration::from_millis(ms));
    }
    let cursor = ctx.guard(find).await?;
    emit_docs(Box::pin(cursor), Vec::new(), tx, &ctx.token, true).await
}

pub(super) async fn run(ctx: ExecCtx, op: Op, tx: EventSender) -> Result<()> {
    let schema_change = op.modifies_schema();
    let result = execute(&ctx, op, &tx).await;
    if schema_change {
        ctx.structures.clear();
    }
    result
}

async fn execute(ctx: &ExecCtx, op: Op, tx: &EventSender) -> Result<()> {
    match op {
        Op::ShowDatabases => {
            let admin = ctx.client.database("admin");
            let reply = ctx
                .guard(admin.run_command(doc! {
                    "listDatabases": 1,
                    "authorizedDatabases": true,
                }))
                .await?;
            let docs = reply
                .get_array("databases")
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_document().cloned())
                .collect();
            emit_vec(docs, ctx, tx).await
        }
        Op::ShowCollections | Op::GetCollectionNames => {
            let cursor = ctx
                .guard(ctx.db().run_cursor_command(doc! {
                    "listCollections": 1,
                    "nameOnly": true,
                    "authorizedCollections": true,
                }))
                .await?;
            let mut docs: Vec<Document> = ctx.guard(cursor.try_collect::<Vec<_>>()).await?;
            docs.sort_by(|a, b| {
                a.get_str("name")
                    .unwrap_or_default()
                    .cmp(b.get_str("name").unwrap_or_default())
            });
            emit_vec(docs, ctx, tx).await
        }
        Op::RunCommand { command, admin } => {
            let db = if admin {
                ctx.client.database("admin")
            } else {
                ctx.db()
            };
            let reply = ctx.guard(db.run_command(command)).await?;
            emit_reply(reply, ctx, tx).await
        }
        Op::CreateCollection { name, options } => {
            let mut command = doc! { "create": name };
            command.extend(options);
            let reply = ctx.guard(ctx.db().run_command(command)).await?;
            emit_reply(reply, ctx, tx).await
        }
        Op::DropDatabase => {
            ctx.guard(ctx.db().drop()).await?;
            emit_vec(vec![doc! { "ok": 1, "dropped": &ctx.database }], ctx, tx).await
        }
        Op::Find(f) => run_find(*f, ctx, tx).await,
        Op::Aggregate {
            collection,
            pipeline,
            allow_disk_use,
        } => {
            let coll = ctx.collection(&collection);
            let mut aggregate = coll.aggregate(pipeline).comment(ctx.tag()).batch_size(500);
            if allow_disk_use {
                aggregate = aggregate.allow_disk_use(true);
            }
            let cursor = ctx.guard(aggregate).await?;
            emit_docs(Box::pin(cursor), Vec::new(), tx, &ctx.token, true).await
        }
        Op::CountDocuments {
            collection,
            filter,
            limit,
            skip,
        } => {
            let coll = ctx.collection(&collection);
            let mut count = coll.count_documents(filter).comment(ctx.tag());
            if let Some(limit) = limit {
                count = count.limit(limit);
            }
            if let Some(skip) = skip {
                count = count.skip(skip);
            }
            let n = ctx.guard(count).await?;
            emit_vec(vec![doc! { "count": n as i64 }], ctx, tx).await
        }
        Op::EstimatedCount { collection } => {
            let n = ctx
                .guard(ctx.collection(&collection).estimated_document_count())
                .await?;
            emit_vec(vec![doc! { "count": n as i64 }], ctx, tx).await
        }
        Op::Distinct {
            collection,
            field,
            filter,
        } => {
            let values = ctx
                .guard(ctx.collection(&collection).distinct(&field, filter))
                .await?;
            let docs = values.into_iter().map(|v| doc! { &field: v }).collect();
            emit_vec(docs, ctx, tx).await
        }
        Op::InsertOne {
            collection,
            document,
        } => {
            ctx.guard(ctx.collection(&collection).insert_one(document))
                .await?;
            emit_affected(tx, 1).await
        }
        Op::InsertMany {
            collection,
            documents,
            ordered,
        } => {
            let result = ctx
                .guard(
                    ctx.collection(&collection)
                        .insert_many(documents)
                        .ordered(ordered),
                )
                .await?;
            emit_affected(tx, result.inserted_ids.len() as u64).await
        }
        Op::Update {
            collection,
            filter,
            update,
            many,
            upsert,
            array_filters,
        } => {
            let coll = ctx.collection(&collection);
            let result = if many {
                let mut action = coll.update_many(filter, update).upsert(upsert);
                if let Some(af) = array_filters {
                    action = action.array_filters(af);
                }
                ctx.guard(action).await?
            } else {
                let mut action = coll.update_one(filter, update).upsert(upsert);
                if let Some(af) = array_filters {
                    action = action.array_filters(af);
                }
                ctx.guard(action).await?
            };
            emit_affected(
                tx,
                result.modified_count + u64::from(result.upserted_id.is_some()),
            )
            .await
        }
        Op::Replace {
            collection,
            filter,
            replacement,
            upsert,
        } => {
            let result = ctx
                .guard(
                    ctx.collection(&collection)
                        .replace_one(filter, replacement)
                        .upsert(upsert),
                )
                .await?;
            emit_affected(
                tx,
                result.modified_count + u64::from(result.upserted_id.is_some()),
            )
            .await
        }
        Op::Delete {
            collection,
            filter,
            many,
        } => {
            let coll = ctx.collection(&collection);
            let deleted = if many {
                ctx.guard(coll.delete_many(filter)).await?.deleted_count
            } else {
                ctx.guard(coll.delete_one(filter)).await?.deleted_count
            };
            emit_affected(tx, deleted).await
        }
        Op::CreateIndex {
            collection,
            keys,
            options,
        } => {
            let mut spec = doc! { "key": &keys };
            if !options.contains_key("name") {
                spec.insert("name", index_name(&keys));
            }
            spec.extend(options);
            let reply = ctx
                .guard(ctx.db().run_command(doc! {
                    "createIndexes": collection,
                    "indexes": [spec],
                }))
                .await?;
            emit_reply(reply, ctx, tx).await
        }
        Op::DropIndex { collection, index } => {
            let reply = ctx
                .guard(ctx.db().run_command(doc! {
                    "dropIndexes": collection,
                    "index": index,
                }))
                .await?;
            emit_reply(reply, ctx, tx).await
        }
        Op::GetIndexes { collection } => {
            let cursor = ctx
                .guard(
                    ctx.db()
                        .run_cursor_command(doc! { "listIndexes": collection }),
                )
                .await?;
            emit_docs(Box::pin(cursor), Vec::new(), tx, &ctx.token, true).await
        }
        Op::Drop { collection } => {
            ctx.guard(ctx.collection(&collection).drop()).await?;
            emit_vec(vec![doc! { "ok": 1, "dropped": collection }], ctx, tx).await
        }
    }
}
