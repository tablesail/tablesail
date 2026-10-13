//! Table structure of a collection: columns inferred by sampling, indexes, options.

use std::sync::Arc;

use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};

use super::convert::{bson_to_json, db_err};
use super::infer::{self, Tally};
use super::{MongoDriver, is_unauthorized, table_kind};
use crate::error::{Error, Result};
use crate::model::{Editability, IndexInfo, TableKind, TableRef, TableStructure};

/// Documents read from the start of a collection, plus the same number picked at random.
const SAMPLE_HALF: i64 = 100;

fn index_info(index: &Document) -> Option<IndexInfo> {
    let name = index.get_str("name").ok()?.to_owned();
    let key = index.get_document("key").ok()?;
    let method = key
        .values()
        .find_map(|v| v.as_str())
        .unwrap_or("btree")
        .to_owned();
    // Text indexes store `_fts`/`_ftsx` keys; the indexed fields are the weights.
    let columns = match index.get_document("weights") {
        Ok(weights) if key.contains_key("_fts") => weights.keys().cloned().collect(),
        _ => key.keys().cloned().collect(),
    };
    Some(IndexInfo {
        primary: name == "_id_",
        unique: index.get_bool("unique").unwrap_or(false) || name == "_id_",
        name,
        columns,
        method: Some(method),
    })
}

impl MongoDriver {
    /// Structure of a collection, from the cache while it is fresh.
    pub(super) async fn structure(&self, table: &TableRef) -> Result<Arc<TableStructure>> {
        if let Some(cached) = self.structures.get(table) {
            return Ok(cached);
        }
        let built = Arc::new(self.build_structure(table).await?);
        self.structures.put(table, built.clone());
        Ok(built)
    }

    async fn build_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let database = self.database(&table.database).name().to_owned();
        let spec = self.collection_spec(&database, &table.name).await?;
        let type_name = spec.get_str("type").unwrap_or("collection").to_owned();
        let kind = table_kind(&type_name);
        let is_view = kind == TableKind::View;

        let (docs, indexes) = tokio::join!(
            self.sample(table, is_view),
            self.indexes(&database, &table.name, is_view)
        );

        let mut tally = Tally::default();
        for d in &docs? {
            tally.add(d);
        }
        let mut inferred = tally.finish();
        if inferred.first().is_none_or(|c| c.name != "_id") && !is_view {
            inferred.insert(0, infer::default_id_column());
        }

        let mut description = doc! { "name": &table.name, "type": &type_name };
        if let Ok(options) = spec.get_document("options")
            && !options.is_empty()
        {
            description.insert("options", options.clone());
        }
        let ddl = serde_json::to_string_pretty(&bson_to_json(&Bson::Document(description))).ok();

        Ok(TableStructure {
            table: TableRef {
                database,
                schema: None,
                name: table.name.clone(),
            },
            kind,
            columns: infer::to_columns(inferred),
            primary_key: vec!["_id".into()],
            indexes: indexes.unwrap_or_default(),
            foreign_keys: Vec::new(),
            editability: if is_view {
                Editability::ReadOnly("Views can't be edited.".into())
            } else {
                Editability::Editable
            },
            ddl,
        })
    }

    /// The `listCollections` entry of one collection.
    async fn collection_spec(&self, database: &str, name: &str) -> Result<Document> {
        let found = self
            .cursor_command(
                database,
                doc! { "listCollections": 1, "filter": { "name": name } },
            )
            .await;
        match found {
            Ok(mut entries) => entries
                .pop()
                .ok_or_else(|| Error::NotFound(format!("collection {database}.{name}"))),
            // Users restricted to some collections cannot run the full listing.
            Err(e) if is_unauthorized(&e) => Ok(doc! { "name": name, "type": "collection" }),
            Err(e) => Err(e),
        }
    }

    async fn sample(&self, table: &TableRef, is_view: bool) -> Result<Vec<Document>> {
        let coll = self.collection(table);
        if is_view {
            return coll
                .find(doc! {})
                .limit(2 * SAMPLE_HALF)
                .await
                .map_err(db_err)?
                .try_collect()
                .await
                .map_err(db_err);
        }
        // The head of the collection gives a stable column order between
        // refreshes; the random half finds fields that only later documents have.
        let head = async {
            coll.find(doc! {})
                .limit(SAMPLE_HALF)
                .await?
                .try_collect::<Vec<Document>>()
                .await
        };
        let random = async {
            coll.aggregate([doc! { "$sample": { "size": SAMPLE_HALF } }])
                .await?
                .try_collect::<Vec<Document>>()
                .await
        };
        let (head, random) = tokio::join!(head, random);
        let mut docs = head.map_err(db_err)?;
        if docs.len() as i64 >= SAMPLE_HALF {
            docs.extend(random.unwrap_or_default());
        }
        Ok(docs)
    }

    async fn indexes(&self, database: &str, name: &str, is_view: bool) -> Result<Vec<IndexInfo>> {
        if is_view {
            return Ok(Vec::new());
        }
        let docs = self
            .cursor_command(database, doc! { "listIndexes": name })
            .await?;
        Ok(docs.iter().filter_map(index_info).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_describe_their_keys() {
        let primary = index_info(&doc! { "v": 2, "key": { "_id": 1 }, "name": "_id_" }).unwrap();
        assert!(primary.primary && primary.unique);
        assert_eq!(primary.method.as_deref(), Some("btree"));

        let compound =
            index_info(&doc! { "key": { "a": 1, "b": -1 }, "name": "a_1_b_-1", "unique": true })
                .unwrap();
        assert_eq!(compound.columns, ["a", "b"]);
        assert!(compound.unique && !compound.primary);

        let text = index_info(&doc! {
            "key": { "_fts": "text", "_ftsx": 1 },
            "name": "t",
            "weights": { "title": 1, "body": 1 },
        })
        .unwrap();
        assert_eq!(text.columns, ["title", "body"]);
        assert_eq!(text.method.as_deref(), Some("text"));
    }
}
