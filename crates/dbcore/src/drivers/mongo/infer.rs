//! Column inference from sampled documents: the union of top-level fields
//! (`_id` first, then by first appearance), typed by the most common BSON type.

use std::collections::HashMap;

use mongodb::bson::{Bson, Document};

use super::convert::{family_of, type_name};
use crate::model::{ColumnInfo, ColumnMeta, TypeFamily};

struct FieldTally {
    name: String,
    present: usize,
    /// (BSON type name, family, count) in order of first appearance.
    types: Vec<(&'static str, TypeFamily, usize)>,
}

#[derive(Default)]
pub struct Tally {
    docs: usize,
    fields: Vec<FieldTally>,
    index: HashMap<String, usize>,
}

/// The inferred type of one column.
pub struct Inferred {
    pub name: String,
    pub data_type: String,
    pub family: TypeFamily,
    pub nullable: bool,
}

impl Tally {
    pub fn add(&mut self, doc: &Document) {
        self.docs += 1;
        for (key, value) in doc {
            let slot = match self.index.get(key) {
                Some(&i) => i,
                None => {
                    self.fields.push(FieldTally {
                        name: key.clone(),
                        present: 0,
                        types: Vec::new(),
                    });
                    self.index.insert(key.clone(), self.fields.len() - 1);
                    self.fields.len() - 1
                }
            };
            let field = &mut self.fields[slot];
            field.present += 1;
            let name = type_name(value);
            match field.types.iter_mut().find(|t| t.0 == name) {
                Some(t) => t.2 += 1,
                None => field.types.push((name, family_of(value), 1)),
            }
        }
    }

    /// Columns with `_id` first, then in order of first appearance.
    pub fn finish(self) -> Vec<Inferred> {
        let docs = self.docs;
        let mut out: Vec<Inferred> = self
            .fields
            .into_iter()
            .map(|f| infer_field(f, docs))
            .collect();
        if let Some(pos) = out.iter().position(|c| c.name == "_id") {
            let id = out.remove(pos);
            out.insert(0, id);
        }
        out
    }
}

fn infer_field(field: FieldTally, docs: usize) -> Inferred {
    let nulls: usize = field
        .types
        .iter()
        .filter(|t| matches!(t.0, "null" | "undefined"))
        .map(|t| t.2)
        .sum();
    let mut types: Vec<(&str, TypeFamily, usize)> = field
        .types
        .into_iter()
        .filter(|t| !matches!(t.0, "null" | "undefined"))
        .collect();
    // int and long mix freely in practice; show them as one type.
    if let (Some(int_at), true) = (
        types.iter().position(|t| t.0 == "int"),
        types.iter().any(|t| t.0 == "long"),
    ) {
        let (_, _, count) = types.remove(int_at);
        if let Some(long) = types.iter_mut().find(|t| t.0 == "long") {
            long.2 += count;
        }
    }
    // Stable sort keeps first-appearance order among equal counts.
    types.sort_by_key(|t| std::cmp::Reverse(t.2));

    let (data_type, family) = match types.as_slice() {
        [] => ("null".to_owned(), TypeFamily::Other),
        [(name, family, _)] => ((*name).to_owned(), *family),
        [(_, family, _), ..] => (
            format!(
                "mixed({})",
                types.iter().map(|t| t.0).collect::<Vec<_>>().join(",")
            ),
            *family,
        ),
    };
    Inferred {
        nullable: field.name != "_id" && (field.present < docs || nulls > 0),
        name: field.name,
        data_type,
        family,
    }
}

pub fn to_columns(inferred: Vec<Inferred>) -> Vec<ColumnInfo> {
    inferred
        .into_iter()
        .map(|c| ColumnInfo {
            is_primary_key: c.name == "_id",
            name: c.name,
            data_type: c.data_type,
            family: c.family,
            nullable: c.nullable,
            default: None,
            is_auto_increment: false,
            is_generated: false,
            enum_values: None,
            comment: None,
        })
        .collect()
}

pub fn to_meta(inferred: Vec<Inferred>) -> Vec<ColumnMeta> {
    inferred
        .into_iter()
        .map(|c| ColumnMeta {
            name: c.name,
            type_name: c.data_type,
            family: c.family,
        })
        .collect()
}

/// Columns of a batch of documents.
pub fn columns_of(docs: &[Document]) -> Vec<ColumnMeta> {
    let mut tally = Tally::default();
    for d in docs {
        tally.add(d);
    }
    to_meta(tally.finish())
}

/// The `_id` column every non-empty collection has.
pub fn default_id_column() -> Inferred {
    Inferred {
        name: "_id".into(),
        data_type: type_name(&Bson::ObjectId(mongodb::bson::oid::ObjectId::new())).into(),
        family: TypeFamily::ObjectId,
        nullable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn unions_fields_and_picks_the_common_type() {
        let docs = [
            doc! { "a": 1, "_id": 1, "b": "x" },
            doc! { "_id": 2, "a": 2i64, "c": 1.5 },
            doc! { "_id": 3, "a": "oops", "b": Bson::Null },
            doc! { "_id": 4, "a": 5 },
        ];
        let mut t = Tally::default();
        docs.iter().for_each(|d| t.add(d));
        let cols = t.finish();
        let names: Vec<_> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["_id", "a", "b", "c"]);
        assert_eq!(cols[0].data_type, "int");
        assert!(!cols[0].nullable);
        assert_eq!(cols[1].data_type, "mixed(long,string)");
        assert_eq!(cols[1].family, TypeFamily::Integer);
        assert!(!cols[1].nullable);
        assert_eq!(cols[2].data_type, "string");
        assert!(cols[2].nullable);
        assert_eq!(cols[3].data_type, "double");
        assert!(cols[3].nullable);
    }
}
