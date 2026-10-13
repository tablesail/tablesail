//! Edits and CSV import: change sets become insertOne / updateOne / deleteOne.

use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Binary, Bson, Document, doc};
use mongodb::{ClientSession, Collection};

use super::convert::{
    Target, cell_to_bson, db_err, is_missing, is_transactions_unsupported, text_to_bson,
    value_to_bson,
};
use super::render::{collection_ref, render_doc};
use super::{MongoDriver, shell};
use crate::error::{Error, Result};
use crate::model::{
    ApplyResult, ChangeSet, ColumnInput, ColumnValue, Editability, RowChange, TableRef,
    TableStructure,
};
use crate::value::Value;

/// Documents per `insert_many` during CSV import.
const IMPORT_BATCH: usize = 1000;
const CONFLICT: &str = "Document was changed or deleted by someone else";

/// One planned write.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Planned {
    Insert { document: Document },
    Update { filter: Document, update: Document },
    Delete { filter: Document },
}

impl Planned {
    /// The shell command that does the same thing.
    pub(super) fn render(&self, collection: &str) -> String {
        let target = collection_ref(collection);
        match self {
            Planned::Insert { document } => {
                format!("{target}.insertOne({})", render_doc(document))
            }
            Planned::Update { filter, update } => format!(
                "{target}.updateOne({}, {})",
                render_doc(filter),
                render_doc(update)
            ),
            Planned::Delete { filter } => format!("{target}.deleteOne({})", render_doc(filter)),
        }
    }
}

/// What to match a loaded value against. Legacy UUIDs (binary subtype 3) read
/// the same as modern ones, so UUIDs match either subtype.
fn match_value(value: &Value) -> Result<Bson> {
    if let Value::Uuid(text) = value
        && let Ok(parsed) = uuid::Uuid::parse_str(text)
    {
        let binary = |subtype| {
            Bson::Binary(Binary {
                subtype,
                bytes: parsed.as_bytes().to_vec(),
            })
        };
        return Ok(Bson::Document(doc! {
            "$in": [binary(BinarySubtype::Uuid), binary(BinarySubtype::UuidOld)]
        }));
    }
    value_to_bson(value)
}

/// `{ _id: <key> }`.
fn key_filter(key: &[ColumnValue]) -> Result<Document> {
    if key.is_empty() {
        return Err(Error::InvalidInput(
            "This document has no _id, so it cannot be edited".into(),
        ));
    }
    let mut filter = Document::new();
    for k in key {
        filter.insert(k.column.clone(), match_value(&k.value)?);
    }
    Ok(filter)
}

/// An equality condition on a value as loaded, for the optimistic check. Values
/// that cannot be compared safely (documents, arrays, truncated data) give none.
fn original_condition(value: &Value) -> Option<Bson> {
    match value {
        Value::Json(_) | Value::TruncatedText { .. } | Value::Time(_) => None,
        Value::Float(f) => f.is_finite().then_some(Bson::Double(*f)),
        Value::Bytes { data, len } if data.len() != *len => None,
        Value::Other { .. } if is_missing(value) => Some(Bson::Document(doc! { "$exists": false })),
        Value::Other { .. } => None,
        v => match_value(v).ok(),
    }
}

fn input_pair(
    columns: &TableStructure,
    input: &ColumnInput,
    original: Option<&Value>,
) -> Result<Option<Bson>> {
    let info = columns.columns.iter().find(|c| c.name == input.column);
    cell_to_bson(&input.value, Target::resolve(original, info))
}

/// Turns a change set into writes, typing each cell by the field's current
/// value or, failing that, the inferred column.
pub(super) fn plan(changes: &ChangeSet, structure: &TableStructure) -> Result<Vec<Planned>> {
    if let Editability::ReadOnly(reason) = &structure.editability {
        return Err(Error::Unsupported(reason.clone()));
    }
    let mut planned = Vec::with_capacity(changes.changes.len());
    for change in &changes.changes {
        match change {
            RowChange::Insert { values } => {
                let mut document = Document::new();
                for input in values {
                    if let Some(bson) = input_pair(structure, input, None)? {
                        document.insert(input.column.clone(), bson);
                    }
                }
                planned.push(Planned::Insert { document });
            }
            RowChange::Update { key, original, set } => {
                let mut filter = key_filter(key)?;
                for o in original {
                    if filter.contains_key(&o.column) {
                        continue;
                    }
                    if let Some(condition) = original_condition(&o.value) {
                        filter.insert(o.column.clone(), condition);
                    }
                }
                let mut set_doc = Document::new();
                let mut unset_doc = Document::new();
                for input in set {
                    let before = original
                        .iter()
                        .find(|o| o.column == input.column)
                        .map(|o| &o.value);
                    match input_pair(structure, input, before)? {
                        Some(bson) => {
                            set_doc.insert(input.column.clone(), bson);
                        }
                        None => {
                            unset_doc.insert(input.column.clone(), "");
                        }
                    }
                }
                let mut update = Document::new();
                if !set_doc.is_empty() {
                    update.insert("$set", set_doc);
                }
                if !unset_doc.is_empty() {
                    update.insert("$unset", unset_doc);
                }
                if !update.is_empty() {
                    planned.push(Planned::Update { filter, update });
                }
            }
            RowChange::Delete { key } => planned.push(Planned::Delete {
                filter: key_filter(key)?,
            }),
        }
    }
    Ok(planned)
}

/// Runs one write; returns the number of documents it touched.
async fn run_planned(
    coll: &Collection<Document>,
    planned: &Planned,
    session: &mut ClientSession,
) -> Result<u64> {
    let conflict = |p: &Planned| Error::Database {
        message: CONFLICT.into(),
        code: None,
        statement: Some(p.render(coll.name())),
    };
    match planned {
        Planned::Insert { document } => {
            coll.insert_one(document)
                .session(&mut *session)
                .await
                .map_err(db_err)?;
            Ok(1)
        }
        Planned::Update { filter, update } => {
            let result = coll
                .update_one(filter.clone(), update.clone())
                .session(&mut *session)
                .await
                .map_err(db_err)?;
            if result.matched_count == 0 {
                return Err(conflict(planned));
            }
            Ok(1)
        }
        Planned::Delete { filter } => {
            let result = coll
                .delete_one(filter.clone())
                .session(&mut *session)
                .await
                .map_err(db_err)?;
            if result.deleted_count == 0 {
                return Err(conflict(planned));
            }
            Ok(1)
        }
    }
}

impl MongoDriver {
    pub(super) async fn apply(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        let structure = self.structure(&changes.table).await?;
        let planned = plan(changes, &structure)?;
        if planned.is_empty() {
            return Ok(ApplyResult {
                statements: 0,
                rows_affected: 0,
            });
        }
        let coll = self.collection(&changes.table);
        let outcome = match self
            .run_batch(&coll, &planned, self.supports_transactions)
            .await
        {
            // The hello reply said transactions work but the server disagrees
            // (e.g. a proxy in front of a standalone): redo without one.
            Err(e) if self.supports_transactions && is_transactions_unsupported(&e) => {
                self.run_batch(&coll, &planned, false).await
            }
            other => other,
        };
        self.structures
            .invalidate(&changes.table.database, &changes.table.name);
        outcome.map(|rows_affected| ApplyResult {
            statements: planned.len() as u64,
            rows_affected,
        })
    }

    /// Applies the writes in order, in one transaction when asked. Without a
    /// transaction the first error stops the run and earlier writes stay applied.
    async fn run_batch(
        &self,
        coll: &Collection<Document>,
        planned: &[Planned],
        transactional: bool,
    ) -> Result<u64> {
        let mut session = self.client.start_session().await.map_err(db_err)?;
        if transactional {
            session.start_transaction().await.map_err(db_err)?;
        }
        let mut affected = 0;
        for (i, write) in planned.iter().enumerate() {
            match run_planned(coll, write, &mut session).await {
                Ok(n) => affected += n,
                Err(e) => {
                    if transactional {
                        let _ = session.abort_transaction().await;
                    }
                    let mut e = e.with_statement(&write.render(coll.name()));
                    if !transactional
                        && i > 0
                        && let Error::Database { message, .. } = &mut e
                    {
                        message.push_str(&format!(
                                " ({i} earlier change{} already applied: this server has no transactions)",
                                if i == 1 { " was" } else { "s were" }
                            ));
                    }
                    return Err(e);
                }
            }
        }
        if transactional {
            session.commit_transaction().await.map_err(db_err)?;
        }
        Ok(affected)
    }

    pub(super) async fn insert_csv(
        &self,
        table: &TableRef,
        columns: &[String],
        rows: Vec<Vec<Option<String>>>,
    ) -> Result<u64> {
        let structure = self.structure(table).await?;
        if let Editability::ReadOnly(reason) = &structure.editability {
            return Err(Error::Unsupported(reason.clone()));
        }
        let targets: Vec<Target> = columns
            .iter()
            .map(|name| {
                structure
                    .columns
                    .iter()
                    .find(|c| &c.name == name)
                    .map_or(Target::Unknown, Target::from_column)
            })
            .collect();

        let coll = self.collection(table);
        let mut inserted = 0u64;
        for chunk in rows.chunks(IMPORT_BATCH) {
            let mut documents = Vec::with_capacity(chunk.len());
            for row in chunk {
                let mut document = Document::new();
                for ((name, target), cell) in columns.iter().zip(&targets).zip(row) {
                    // NULL and blank non-text cells leave the field out.
                    let Some(text) = cell else { continue };
                    if text.is_empty() && *target != Target::Str && *target != Target::Unknown {
                        continue;
                    }
                    let value = match target {
                        // Structured columns accept JSON in CSV cells.
                        Target::Container => text_to_bson(text, Target::Container)
                            .or_else(|_| shell::parse_value(text))?,
                        t => text_to_bson(text, *t)?,
                    };
                    document.insert(name.clone(), value);
                }
                documents.push(document);
            }
            match coll.insert_many(documents).await {
                Ok(result) => inserted += result.inserted_ids.len() as u64,
                Err(e) => {
                    self.structures.invalidate(&table.database, &table.name);
                    let mut err = db_err(e);
                    if let (Error::Database { message, .. }, true) = (&mut err, inserted > 0) {
                        message.push_str(&format!(
                            " ({inserted} rows were inserted before this error)"
                        ));
                    }
                    return Err(err);
                }
            }
        }
        self.structures.invalidate(&table.database, &table.name);
        Ok(inserted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColumnInfo, ColumnValue, TableKind, TypeFamily};
    use crate::value::CellInput;

    fn column(name: &str, data_type: &str, family: TypeFamily) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            family,
            nullable: true,
            default: None,
            is_primary_key: name == "_id",
            is_auto_increment: false,
            is_generated: false,
            enum_values: None,
            comment: None,
        }
    }

    fn structure() -> TableStructure {
        TableStructure {
            table: TableRef {
                database: "d".into(),
                schema: None,
                name: "users".into(),
            },
            kind: TableKind::Collection,
            columns: vec![
                column("_id", "objectId", TypeFamily::ObjectId),
                column("age", "long", TypeFamily::Integer),
                column("name", "string", TypeFamily::Text),
            ],
            primary_key: vec!["_id".into()],
            indexes: vec![],
            foreign_keys: vec![],
            editability: Editability::Editable,
            ddl: None,
        }
    }

    fn change_set(changes: Vec<RowChange>) -> ChangeSet {
        ChangeSet {
            table: structure().table,
            changes,
        }
    }

    fn id() -> Vec<ColumnValue> {
        vec![ColumnValue {
            column: "_id".into(),
            value: Value::ObjectId("65f000000000000000000001".into()),
        }]
    }

    #[test]
    fn updates_set_unset_and_check_the_original() {
        let planned = plan(
            &change_set(vec![RowChange::Update {
                key: id(),
                original: vec![ColumnValue {
                    column: "age".into(),
                    value: Value::Int(7),
                }],
                set: vec![
                    ColumnInput {
                        column: "age".into(),
                        value: CellInput::Text("8".into()),
                    },
                    ColumnInput {
                        column: "name".into(),
                        value: CellInput::Default,
                    },
                ],
            }]),
            &structure(),
        )
        .unwrap();
        let Planned::Update { filter, update } = &planned[0] else {
            panic!()
        };
        assert!(matches!(filter.get("_id"), Some(Bson::ObjectId(_))));
        assert_eq!(filter.get("age"), Some(&Bson::Int32(7)));
        // The column is `long`, so the edit stays a long.
        assert_eq!(
            update,
            &doc! { "$set": { "age": Bson::Int64(8) }, "$unset": { "name": "" } }
        );
        assert_eq!(
            planned[0].render("users"),
            "db.users.updateOne({ _id: ObjectId(\"65f000000000000000000001\"), age: 7 }, { $set: { age: NumberLong(\"8\") }, $unset: { name: \"\" } })"
        );
    }

    #[test]
    fn inserts_skip_defaults_and_type_by_column() {
        let planned = plan(
            &change_set(vec![
                RowChange::Insert {
                    values: vec![
                        ColumnInput {
                            column: "_id".into(),
                            value: CellInput::Default,
                        },
                        ColumnInput {
                            column: "age".into(),
                            value: CellInput::Text("30".into()),
                        },
                        ColumnInput {
                            column: "name".into(),
                            value: CellInput::Text("Ann".into()),
                        },
                    ],
                },
                RowChange::Delete { key: id() },
            ]),
            &structure(),
        )
        .unwrap();
        assert_eq!(
            planned[0],
            Planned::Insert {
                document: doc! { "age": Bson::Int64(30), "name": "Ann" }
            }
        );
        assert_eq!(
            planned[1].render("users"),
            "db.users.deleteOne({ _id: ObjectId(\"65f000000000000000000001\") })"
        );
    }

    #[test]
    fn views_are_read_only() {
        let mut s = structure();
        s.editability = Editability::ReadOnly("Views can't be edited.".into());
        assert!(plan(&change_set(vec![RowChange::Delete { key: id() }]), &s).is_err());
    }
}
