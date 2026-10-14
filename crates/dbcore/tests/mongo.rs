//! MongoDB: catalog, structure inference, browsing, types, edits, shell statements,
//! cancelling, export and import against the docker server.
//!
//! The docker Mongo is a standalone server, so there are no transactions: a failing
//! change set keeps what it already applied (the driver says so in its message).
#![recursion_limit = "1024"]

#[macro_use]
mod common;

use std::time::{Duration, Instant};

use common::*;
use dbcore::driver::CursorEvent;
use dbcore::model::{
    ChangeSet, ColumnInput, ColumnValue, Editability, FilterOp, RowChange, TableKind, TypeFamily,
};
use dbcore::value::CellInput;
use dbcore::{Engine, Error, Value};
use mongodb::bson::{self, Bson, Document, doc, oid::ObjectId};

fn int(i: i64) -> Value {
    Value::Int(i)
}

fn text(s: &str) -> Value {
    Value::Text(s.to_owned())
}

/// What the grid shows for a field a document does not have.
fn missing() -> Value {
    Value::Other {
        type_name: "missing".into(),
        text: String::new(),
    }
}

fn txt(s: &str) -> CellInput {
    CellInput::Text(s.to_owned())
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Document `i` of the items fixture: `_id` is the number; `label` is a string, null
/// (`i % 10 == 0`) or absent (`i % 10 == 5`).
fn item_doc(i: i64) -> Document {
    let mut d = doc! {
        "_id": i as i32,
        "name": format!("item {i:05}"),
        "grp": (i % 7) as i32,
    };
    match i % 10 {
        0 => {
            d.insert("label", Bson::Null);
        }
        5 => {}
        _ => {
            d.insert("label", format!("lbl-{}", i % 13));
        }
    }
    d
}

fn item_label(i: i64) -> Value {
    match i % 10 {
        0 => Value::Null,
        5 => missing(),
        _ => text(&format!("lbl-{}", i % 13)),
    }
}

fn item_row(i: i64) -> Row {
    vec![
        int(i),
        text(&format!("item {i:05}")),
        int(i % 7),
        item_label(i),
    ]
}

async fn make_items(c: &Ctx, suffix: &str, n: i64) -> String {
    let name = c.scratch(suffix);
    let coll = c.mongo_coll(&name);
    let ids: Vec<i64> = (1..=n).collect();
    for chunk in ids.chunks(1000) {
        coll.insert_many(chunk.iter().map(|i| item_doc(*i)))
            .await
            .unwrap();
    }
    if n == 0 {
        c.mongo_db().create_collection(&name).await.unwrap();
    }
    name
}

async fn run_mongo(c: &Ctx, statement: &str) -> Vec<CursorEvent> {
    c.run(statement).await
}

// ---------------------------------------------------------------------------
// Connecting and the catalog
// ---------------------------------------------------------------------------

async fn connect_ping_and_catalog(c: &Ctx) {
    let version = c.driver.server_version().await.unwrap();
    assert!(version.starts_with("MongoDB "), "{version}");
    assert_eq!(c.driver.engine(), Engine::Mongodb);
    assert_eq!(c.driver.dialect(), dbcore::Dialect::Mongo);
    // A trivial round trip on the shell path.
    let events = run_mongo(c, "db.runCommand({ ping: 1 })").await;
    assert_eq!(sets_of(&events)[0].rows[0][0], Value::Float(1.0));

    let databases = c.driver.list_databases().await.unwrap();
    assert!(databases.contains(&DB.to_owned()), "{databases:?}");
    let mut sorted = databases.clone();
    sorted.sort();
    assert_eq!(databases, sorted);
    assert!(c.driver.list_schemas(DB).await.unwrap().is_empty());

    // Scratch collection, view and capped collection show up with their kinds.
    let coll = make_items(c, "catalog", 12).await;
    let view = c.scratch("catalog_view");
    c.mongo_db()
        .run_command(
            doc! { "create": &view, "viewOn": &coll, "pipeline": [ { "$match": { "grp": 1 } } ] },
        )
        .await
        .unwrap();
    let capped = c.scratch("catalog_capped");
    c.mongo_db()
        .run_command(doc! { "create": &capped, "capped": true, "size": 65536 })
        .await
        .unwrap();
    let tables = c.driver.list_tables(DB, None).await.unwrap();
    let find = |n: &str| {
        tables
            .iter()
            .find(|t| t.table.name == n)
            .unwrap_or_else(|| panic!("{n} missing"))
    };
    assert_eq!(find(&coll).kind, TableKind::Collection);
    assert_eq!(find(&coll).estimated_rows, Some(12));
    assert_eq!(find(&coll).table.database, DB);
    assert_eq!(find(&view).kind, TableKind::View);
    assert_eq!(find(&view).estimated_rows, None);
    assert_eq!(find(&capped).kind, TableKind::Collection);
    assert_eq!(find(&capped).estimated_rows, Some(0));
    assert!(!tables.iter().any(|t| t.table.name.starts_with("system.")));
    let names: Vec<&str> = tables.iter().map(|t| t.table.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "collections come back sorted");

    // The seeded collections.
    for name in [
        "all_types",
        "big_collection",
        "heterogeneous",
        "deep_nesting",
        "select",
        "with space",
        "dots.in.name",
        "customer_totals",
        "capped_log",
        "metrics",
        "empty_collection",
        "seed_done",
        "wide_document",
        "ñame 🔑",
        "customers",
        "orders",
        "validated",
    ] {
        find(name);
    }
    assert_eq!(find("customer_totals").kind, TableKind::View);
    assert_eq!(find("metrics").kind, TableKind::TimeSeries);
    assert_eq!(find("empty_collection").estimated_rows, Some(0));
    let big = find("big_collection").estimated_rows.unwrap();
    assert_eq!(big, 1_000_000);
    // A database that does not exist has no collections.
    assert!(
        c.driver
            .list_tables("no_such_db_xyz", None)
            .await
            .unwrap()
            .is_empty()
    );
}
each_engine!(connect_ping_and_catalog, mongo);

#[tokio::test]
async fn mongo_connection_failures_are_prompt() {
    // Nothing listens on port 1.
    let started = Instant::now();
    let err = connect_driver(Engine::Mongodb, None, |cfg| {
        cfg.host = "127.0.0.1".into();
        cfg.port = 1;
        cfg.url = Some("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=500&connectTimeoutMS=500&directConnection=true".into());
    })
    .await
    .err()
    .expect("connecting to a closed port must fail");
    assert!(matches!(err, Error::Connection(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    // A malformed URL.
    let err = connect_driver(Engine::Mongodb, None, |cfg| {
        cfg.url = Some("not a mongo url".into())
    })
    .await
    .err()
    .unwrap();
    assert!(
        matches!(
            err,
            Error::Database { .. } | Error::InvalidInput(_) | Error::Connection(_)
        ),
        "{err:?}"
    );
}

async fn session_open_works_for_mongo(c: &Ctx) {
    let (cfg, password) = config_for(Engine::Mongodb, None);
    let session = dbcore::Session::open(
        cfg,
        dbcore::model::Secrets {
            db_password: password,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        session.server_version,
        c.driver.server_version().await.unwrap()
    );
    session.close().await;
}
each_engine!(session_open_works_for_mongo, mongo);

// ---------------------------------------------------------------------------
// Structure
// ---------------------------------------------------------------------------

async fn structure_is_inferred_from_documents(c: &Ctx) {
    let name = c.scratch("infer");
    let coll = c.mongo_coll(&name);
    let oid = ObjectId::parse_str("65f000000000000000000001").unwrap();
    coll.insert_many([
        doc! { "_id": oid, "s": "text", "i": 1, "l": 2i64, "d": 1.5, "b": true, "at": bson::DateTime::from_millis(0),
               "dec": bson::Decimal128::from_bytes([0; 16]), "bin": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
               "obj": { "k": 1 }, "arr": [1, 2], "mixed": 1, "only_here": "x" },
        doc! { "s": "more", "i": 2, "l": 3i64, "d": 2.5, "b": false, "mixed": "text", "nul": Bson::Null },
        doc! { "s": "more", "mixed": "again" },
        doc! { "mixed": 3.5 },
    ])
    .await
    .unwrap();
    coll.create_index(
        mongodb::IndexModel::builder()
            .keys(doc! { "s": 1, "i": -1 })
            .options(
                mongodb::options::IndexOptions::builder()
                    .unique(true)
                    .name("s_i_unique".to_owned())
                    .build(),
            )
            .build(),
    )
    .await
    .unwrap();
    coll.create_index(mongodb::IndexModel::builder().keys(doc! { "d": 1 }).build())
        .await
        .unwrap();
    coll.create_index(
        mongodb::IndexModel::builder()
            .keys(doc! { "s": "text" })
            .build(),
    )
    .await
    .unwrap();

    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    assert_eq!(s.kind, TableKind::Collection);
    assert_eq!(s.primary_key, ["_id"]);
    assert_eq!(s.editability, Editability::Editable);
    assert!(s.foreign_keys.is_empty());
    let names: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[0], "_id", "_id is always first");
    for n in [
        "s",
        "i",
        "l",
        "d",
        "b",
        "at",
        "dec",
        "bin",
        "obj",
        "arr",
        "mixed",
        "only_here",
        "nul",
    ] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    assert_eq!(names.len(), 14);
    let col = |n: &str| s.columns.iter().find(|c| c.name == n).unwrap();
    assert!(col("_id").is_primary_key && !col("_id").nullable);
    assert_eq!(col("_id").family, TypeFamily::ObjectId);
    assert_eq!(col("_id").data_type, "objectId");
    assert_eq!(
        (col("s").family, col("s").data_type.as_str()),
        (TypeFamily::Text, "string")
    );
    assert_eq!(col("i").family, TypeFamily::Integer);
    assert_eq!(col("l").family, TypeFamily::Integer);
    assert_eq!(col("d").family, TypeFamily::Float);
    assert_eq!(col("b").family, TypeFamily::Boolean);
    assert_eq!(col("at").family, TypeFamily::DateTime);
    assert_eq!(col("dec").family, TypeFamily::Decimal);
    assert_eq!(col("bin").family, TypeFamily::Binary);
    assert_eq!(col("obj").family, TypeFamily::Document);
    assert_eq!(col("arr").family, TypeFamily::Array);
    // Fields missing from some documents (or null) are nullable; present everywhere are not.
    assert!(col("s").nullable && col("only_here").nullable && col("nul").nullable);
    assert!(!col("mixed").nullable);
    // A field with several types is reported as mixed, led by the most common.
    assert!(
        col("mixed").data_type.starts_with("mixed("),
        "{}",
        col("mixed").data_type
    );
    assert!(col("mixed").data_type.contains("string"));
    // Indexes.
    let id_index = s.indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!(id_index.name, "_id_");
    assert!(id_index.unique);
    let unique = s.indexes.iter().find(|i| i.name == "s_i_unique").unwrap();
    assert!(unique.unique && !unique.primary);
    assert_eq!(unique.columns, ["s", "i"]);
    let plain = s.indexes.iter().find(|i| i.columns == ["d"]).unwrap();
    assert!(!plain.unique);
    let text_index = s
        .indexes
        .iter()
        .find(|i| i.method.as_deref() == Some("text"))
        .unwrap();
    assert_eq!(text_index.columns, ["s"]);
    // Collection options come back as JSON.
    let ddl: serde_json::Value = serde_json::from_str(s.ddl.as_deref().unwrap()).unwrap();
    assert_eq!(ddl["name"], name.as_str());
    assert_eq!(ddl["type"], "collection");
}
each_engine!(structure_is_inferred_from_documents, mongo);

async fn structure_of_views_capped_and_missing_collections(c: &Ctx) {
    let coll = make_items(c, "struct_src", 5).await;
    let view = c.scratch("struct_view");
    c.mongo_db()
        .run_command(doc! { "create": &view, "viewOn": &coll, "pipeline": [ { "$project": { "name": 1 } } ] })
        .await
        .unwrap();
    let s = c.driver.table_structure(&c.tref(&view)).await.unwrap();
    assert_eq!(s.kind, TableKind::View);
    assert_eq!(
        s.editability,
        Editability::ReadOnly("Views can't be edited.".into())
    );
    assert!(s.indexes.is_empty());
    assert!(s.columns.iter().any(|c| c.name == "name"));
    assert!(matches!(
        c.driver
            .apply_changes(&ChangeSet {
                table: c.tref(&view),
                changes: vec![RowChange::Delete {
                    key: vec![ColumnValue {
                        column: "_id".into(),
                        value: int(1)
                    }]
                }],
            })
            .await,
        Err(Error::Unsupported(_))
    ));
    // Browsing a view works.
    assert_eq!(c.page(&c.browse(&view), 0, 10).await.rows.len(), 5);

    let capped = c.scratch("struct_capped");
    c.mongo_db()
        .run_command(doc! { "create": &capped, "capped": true, "size": 65536, "max": 100 })
        .await
        .unwrap();
    c.mongo_coll(&capped)
        .insert_one(doc! { "m": "x" })
        .await
        .unwrap();
    let s = c.driver.table_structure(&c.tref(&capped)).await.unwrap();
    let ddl: serde_json::Value = serde_json::from_str(s.ddl.as_deref().unwrap()).unwrap();
    assert_eq!(ddl["options"]["capped"], true);
    assert_eq!(ddl["options"]["max"], 100);

    // An empty collection still has an _id column.
    let empty = make_items(c, "struct_empty", 0).await;
    let s = c.driver.table_structure(&c.tref(&empty)).await.unwrap();
    assert_eq!(s.columns.len(), 1);
    assert_eq!(s.columns[0].name, "_id");
    assert_eq!(s.primary_key, ["_id"]);
    let page = c.page(&c.browse(&empty), 0, 10).await;
    assert!(page.rows.is_empty());
    assert_eq!(page.columns.len(), 1);

    let err = c
        .driver
        .table_structure(&c.tref("no_such_collection_xyz"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}
each_engine!(structure_of_views_capped_and_missing_collections, mongo);

async fn structure_notices_fields_that_appear_late(c: &Ctx) {
    // The structure samples the first 100 documents (plus a random 100): a field first
    // seen in the 100th document is always found, one in the 150th only by chance.
    let name = c.scratch("late_field");
    let coll = c.mongo_coll(&name);
    let mut docs: Vec<Document> = (1..=150).map(|i| doc! { "_id": i, "a": i }).collect();
    docs[99].insert("late", "found");
    docs[149].insert("very_late", "browse only");
    coll.insert_many(docs).await.unwrap();
    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    let late = s
        .columns
        .iter()
        .find(|c| c.name == "late")
        .expect("late column");
    assert!(late.nullable);
    // A page that has a field shows it even when the sample missed it.
    let page = c.page(&c.browse(&name), 140, 20).await;
    assert!(page.columns.iter().any(|c| c.name == "very_late"));
    let idx = page
        .columns
        .iter()
        .position(|c| c.name == "very_late")
        .unwrap();
    assert_eq!(
        page.rows
            .iter()
            .filter(|r| r[idx] == text("browse only"))
            .count(),
        1
    );
    assert_eq!(page.rows.iter().filter(|r| r[idx] == missing()).count(), 9);
}
each_engine!(structure_notices_fields_that_appear_late, mongo);

async fn seeded_collections_all_browse(c: &Ctx) {
    // (Other tests' scratch collections come and go while this runs.)
    let tables: Vec<_> = c
        .driver
        .list_tables(DB, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| !t.table.name.starts_with("tst_"))
        .collect();
    assert!(tables.len() >= 15);
    for info in &tables {
        let name = &info.table.name;
        let s = c
            .driver
            .table_structure(&c.tref(name))
            .await
            .unwrap_or_else(|e| panic!("structure of {name}: {e:?}"));
        assert!(!s.columns.is_empty(), "{name}");
        let page = c
            .driver
            .browse_page(&c.browse(name), 0, 25)
            .await
            .unwrap_or_else(|e| panic!("browse {name}: {e:?}"));
        for row in &page.rows {
            assert_eq!(row.len(), page.columns.len(), "{name}");
        }
        c.driver
            .count_rows(&c.browse(name))
            .await
            .unwrap_or_else(|e| panic!("count {name}: {e:?}"));
    }
    // A few details of the nasty collections.
    let all_types = c.page(&c.browse("all_types"), 0, 10).await;
    let col = |n: &str| {
        all_types
            .columns
            .iter()
            .position(|c| c.name == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    assert_eq!(all_types.rows[0][col("int64")], int(9_007_199_254_740_993));
    assert_eq!(all_types.rows[2][col("int64")], int(i64::MAX));
    assert!(
        matches!(&all_types.rows[0][col("decimal128")], Value::Decimal(d) if d.starts_with("12345.6789012345678901234567890"))
    );
    assert!(
        matches!(&all_types.rows[1][col("double")], Value::Float(f) if *f == f64::NEG_INFINITY)
    );
    assert!(matches!(&all_types.rows[2][col("double")], Value::Float(f) if f.is_nan()));
    assert_eq!(
        all_types.rows[0][col("objectId")],
        Value::ObjectId("65f0000000000000000000ff".into())
    );
    assert_eq!(
        all_types.rows[0][col("uuid")],
        Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
    );
    assert!(matches!(
        &all_types.rows[2][col("bigString")],
        Value::TruncatedText { len: 1_048_576, .. }
    ));
    assert_eq!(
        all_types.rows[1][col("date")],
        Value::DateTime("0001-01-01T00:00:00.000Z".into())
    );
    assert_eq!(
        all_types.rows[2][col("date")],
        Value::DateTime("9999-12-31T23:59:59.999Z".into())
    );
    // Documents of different shapes in one collection: gaps are "missing", not NULL.
    let h = c.page(&c.browse("heterogeneous"), 0, 20).await;
    assert_eq!(h.rows.len(), 7);
    let only_here = h.columns.iter().position(|c| c.name == "onlyHere").unwrap();
    assert_eq!(
        h.rows
            .iter()
            .filter(|r| r[only_here] == Value::Bool(true))
            .count(),
        1
    );
    assert!(h.rows.iter().filter(|r| r[only_here] == missing()).count() == 6);
    // Each `_id` kind is shown as its own type.
    let ids_shown: Vec<&Value> = h.rows.iter().map(|r| &r[0]).collect();
    assert!(ids_shown.iter().any(|v| matches!(v, Value::Uuid(_))));
    assert!(ids_shown.iter().any(|v| matches!(v, Value::ObjectId(_))));
    assert!(ids_shown.iter().any(|v| matches!(v, Value::Json(_))));
    assert!(
        ids_shown
            .iter()
            .any(|v| matches!(v, Value::Text(t) if t == "string-id"))
    );
}
each_engine!(seeded_collections_all_browse, mongo);

// ---------------------------------------------------------------------------
// Browsing
// ---------------------------------------------------------------------------

async fn paging_sorting_and_projection(c: &Ctx) {
    let t = make_items(c, "paging", 250).await;
    let req = c.browse(&t);
    let page = c.page(&req, 0, 10).await;
    assert_eq!(page.rows, (1..=10).map(item_row).collect::<Vec<_>>());
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["_id", "name", "grp", "label"]);
    assert_eq!(page.columns[0].family, TypeFamily::Integer);
    assert!(
        page.query.starts_with(&format!("db.{t}.find(")),
        "{}",
        page.query
    );
    assert!(page.query.contains(".limit(10)"));
    assert_eq!(
        c.page_ids(&req, 10, 10).await,
        (11..=20).collect::<Vec<_>>()
    );
    assert_eq!(
        c.page_ids(&req, 245, 10).await,
        (246..=250).collect::<Vec<_>>()
    );
    assert_eq!(c.page_ids(&req, 250, 10).await, Vec::<i64>::new());
    assert_eq!(c.page_ids(&req, 5000, 10).await, Vec::<i64>::new());
    let page = c.page(&req, 20, 5).await;
    assert_eq!(page.offset, 20);
    assert_eq!(ids(&page.rows, 0), [21, 22, 23, 24, 25]);
    // limit 0 is "no limit" for Mongo, so ask for at least one.
    assert_eq!(c.page_ids(&req, 249, 1).await, [250]);

    // Sorting: single, multiple, descending; `_id` breaks ties.
    let mut sorted = c.browse(&t);
    sorted.sort = vec![sort("_id", true)];
    assert_eq!(c.page_ids(&sorted, 0, 3).await, [250, 249, 248]);
    sorted.sort = vec![sort("grp", false), sort("_id", true)];
    let mut expected: Vec<i64> = (1..=250).collect();
    expected.sort_by_key(|&i| (i % 7, -i));
    assert_eq!(c.page_ids(&sorted, 0, 1000).await, expected);
    sorted.sort = vec![sort("name", true)];
    assert_eq!(c.page_ids(&sorted, 0, 2).await, [250, 249]);

    // Projection always keeps `_id`.
    let mut proj = c.browse(&t);
    proj.columns = vec!["name".into()];
    let page = c.page(&proj, 0, 3).await;
    assert_eq!(
        page.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["_id", "name"]
    );
    assert_eq!(page.rows[0], vec![int(1), text("item 00001")]);
}
each_engine!(paging_sorting_and_projection, mongo);

async fn deep_paging_through_10k_documents(c: &Ctx) {
    let n = 10_000;
    let t = make_items(c, "deep", n).await;
    let req = c.browse(&t);
    for size in [500u32, 777] {
        let mut all = Vec::new();
        let mut offset = 0u64;
        loop {
            let ids = c.page_ids(&req, offset, size).await;
            if ids.is_empty() {
                break;
            }
            offset += ids.len() as u64;
            all.extend(ids);
        }
        assert_eq!(all, (1..=n).collect::<Vec<_>>(), "page size {size}");
    }
    // Sorting by a column with many equal values still tiles the collection.
    for descending in [false, true] {
        let mut by_grp = c.browse(&t);
        by_grp.sort = vec![sort("grp", descending)];
        let mut all = Vec::new();
        for page in 0..(n as u64 / 250) {
            all.extend(c.page_ids(&by_grp, page * 250, 250).await);
        }
        let mut seen = all.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen,
            (1..=n).collect::<Vec<_>>(),
            "descending: {descending}"
        );
        assert_eq!(all.len() as i64, n);
    }
    assert_eq!(
        c.page_ids(&req, 9_990, 100).await,
        (9_991..=10_000).collect::<Vec<_>>()
    );
    assert_eq!(c.page_ids(&req, 10_000, 1).await, Vec::<i64>::new());
    let mut desc = c.browse(&t);
    desc.sort = vec![sort("_id", true)];
    assert_eq!(c.page_ids(&desc, 9_995, 10).await, [5, 4, 3, 2, 1]);
    assert_eq!(
        c.page(&req, 7_321, 4).await.rows,
        (7_322..=7_325).map(item_row).collect::<Vec<_>>()
    );
    let started = Instant::now();
    let _ = c.page(&req, 0, 100).await;
    let _ = c.page(&req, 9_900, 100).await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}
each_engine!(deep_paging_through_10k_documents, mongo);

fn expect(n: i64, pred: impl Fn(i64) -> bool) -> Vec<i64> {
    (1..=n).filter(|&i| pred(i)).collect()
}

fn label_of(i: i64) -> Option<String> {
    match i % 10 {
        0 | 5 => None,
        _ => Some(format!("lbl-{}", i % 13)),
    }
}

async fn check(
    c: &Ctx,
    t: &str,
    filters: Vec<dbcore::model::Filter>,
    match_all: bool,
    expected: Vec<i64>,
) {
    let mut req = c.browse(t);
    req.filters = filters.clone();
    req.match_all = match_all;
    assert_eq!(
        c.page_ids(&req, 0, 5000).await,
        expected,
        "{filters:?} (match_all {match_all})"
    );
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!(
        count.exact,
        Some(expected.len() as u64),
        "count for {filters:?}"
    );
    let mut all = ids(&c.all_rows(&req).await, 0);
    all.sort();
    assert_eq!(all, expected, "browse_all for {filters:?}");
}

async fn filters_every_operator(c: &Ctx) {
    let n = 100;
    let t = make_items(c, "filters", n).await;
    use FilterOp::*;
    // Comparisons are typed by the column: numbers compare as numbers.
    check(
        c,
        &t,
        vec![filter("grp", Eq, Some("3"))],
        true,
        expect(n, |i| i % 7 == 3),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Neq, Some("3"))],
        true,
        expect(n, |i| i % 7 != 3),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Lt, Some("2"))],
        true,
        expect(n, |i| i % 7 < 2),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Lte, Some("2"))],
        true,
        expect(n, |i| i % 7 <= 2),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Gt, Some("5"))],
        true,
        expect(n, |i| i % 7 > 5),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Gte, Some("5"))],
        true,
        expect(n, |i| i % 7 >= 5),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("_id", Gt, Some("9"))],
        true,
        expect(n, |i| i > 9),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Eq, Some("item 00042"))],
        true,
        vec![42],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Lt, Some("item 00010"))],
        true,
        expect(n, |i| i < 10),
    )
    .await;
    // Text patterns, case-insensitive; special characters are literal.
    check(
        c,
        &t,
        vec![filter("name", Contains, Some("tem 0001"))],
        true,
        expect(n, |i| (10..=19).contains(&i)),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Contains, Some("ITEM 0001"))],
        true,
        expect(n, |i| (10..=19).contains(&i)),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", StartsWith, Some("item 0005"))],
        true,
        expect(n, |i| (50..=59).contains(&i)),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", EndsWith, Some("7"))],
        true,
        expect(n, |i| i % 10 == 7),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Contains, Some("."))],
        true,
        vec![],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Contains, Some("("))],
        true,
        vec![],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Like, Some("item 000_5"))],
        true,
        expect(n, |i| i % 10 == 5 && i < 100),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("name", Like, Some("%99"))],
        true,
        vec![99],
    )
    .await;
    // NOT contains also keeps documents whose field is null or absent.
    check(
        c,
        &t,
        vec![filter("label", NotContains, Some("lbl-1"))],
        true,
        expect(n, |i| {
            !label_of(i).is_some_and(|l| l.to_lowercase().contains("lbl-1"))
        }),
    )
    .await;
    // Patterns on numbers compare their text.
    check(
        c,
        &t,
        vec![filter("_id", Contains, Some("9"))],
        true,
        expect(n, |i| i.to_string().contains('9')),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("_id", StartsWith, Some("1"))],
        true,
        expect(n, |i| i.to_string().starts_with('1')),
    )
    .await;
    // IN lists, typed per column; quoted items; whitespace.
    check(
        c,
        &t,
        vec![filter("_id", In, Some("1,2,3"))],
        true,
        vec![1, 2, 3],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("_id", In, Some(" 3 , 1 ,3, 2 "))],
        true,
        vec![1, 2, 3],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("_id", NotIn, Some("1,2,3"))],
        true,
        (4..=n).collect(),
    )
    .await;
    check(
        c,
        &t,
        vec![filter(
            "name",
            In,
            Some("item 00001,'item 00002',\"item 00003\",nope"),
        )],
        true,
        vec![1, 2, 3],
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", In, Some("0,6"))],
        true,
        expect(n, |i| i % 7 == 0 || i % 7 == 6),
    )
    .await;
    // Null checks cover both null and absent fields.
    check(
        c,
        &t,
        vec![filter("label", IsNull, None)],
        true,
        expect(n, |i| label_of(i).is_none()),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("label", IsNotNull, None)],
        true,
        expect(n, |i| label_of(i).is_some()),
    )
    .await;
    check(c, &t, vec![filter("_id", IsNull, None)], true, vec![]).await;
    // Equality on a nullable field does not match null/absent.
    check(
        c,
        &t,
        vec![filter("label", Eq, Some("lbl-1"))],
        true,
        expect(n, |i| label_of(i).as_deref() == Some("lbl-1")),
    )
    .await;
    // AND / OR.
    check(
        c,
        &t,
        vec![filter("grp", Eq, Some("1")), filter("label", IsNull, None)],
        true,
        expect(n, |i| i % 7 == 1 && label_of(i).is_none()),
    )
    .await;
    check(
        c,
        &t,
        vec![filter("grp", Eq, Some("1")), filter("label", IsNull, None)],
        false,
        expect(n, |i| i % 7 == 1 || label_of(i).is_none()),
    )
    .await;
    // Nothing matches.
    check(c, &t, vec![filter("grp", Eq, Some("99"))], true, vec![]).await;
    // Filtered counts are exact; unfiltered counts are the collection's estimate.
    let count = c.driver.count_rows(&c.browse(&t)).await.unwrap();
    assert_eq!((count.exact, count.estimate), (None, Some(100)));
}
each_engine!(filters_every_operator, mongo);

async fn filters_on_typed_fields(c: &Ctx) {
    let name = c.scratch("typed_filters");
    let coll = c.mongo_coll(&name);
    let oid = |n: u8| ObjectId::parse_str(format!("65f0000000000000000000{n:02x}")).unwrap();
    coll.insert_many((1..=5).map(|i| {
        doc! {
            "_id": oid(i as u8),
            "n": i as i32,
            "big": 9_007_199_254_740_990i64 + i,
            "f": i as f64 + 0.5,
            "ok": i % 2 == 0,
            "at": bson::DateTime::from_millis(1_704_067_200_000 + i * 86_400_000),
            "dec": bson::Decimal128::from_bytes(bson::Decimal128::from_bytes([0; 16]).bytes()),
            "u": bson::Uuid::parse_str(format!("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a{i:02}")).unwrap(),
            "tag": format!("tag-{i}"),
        }
    }))
    .await
    .unwrap();
    use FilterOp::*;
    let pick = |filters: Vec<dbcore::model::Filter>| {
        let mut req = c.browse(&name);
        req.filters = filters;
        req.sort = vec![sort("n", false)];
        async move {
            c.page(&req, 0, 100)
                .await
                .rows
                .iter()
                .map(|r| match &r[1] {
                    Value::Int(i) => *i,
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>()
        }
    };
    // ObjectId by its hex text.
    assert_eq!(
        pick(vec![filter("_id", Eq, Some("65f000000000000000000003"))]).await,
        [3]
    );
    assert_eq!(
        pick(vec![filter(
            "_id",
            In,
            Some("65f000000000000000000001, 65f000000000000000000005")
        )])
        .await,
        [1, 5]
    );
    // Int64 beyond the JavaScript safe range.
    assert_eq!(
        pick(vec![filter("big", Eq, Some("9007199254740993"))]).await,
        [3]
    );
    assert_eq!(
        pick(vec![filter("big", Gt, Some("9007199254740993"))]).await,
        [4, 5]
    );
    // Doubles, booleans.
    assert_eq!(pick(vec![filter("f", Gte, Some("3.5"))]).await, [3, 4, 5]);
    assert_eq!(pick(vec![filter("ok", Eq, Some("true"))]).await, [2, 4]);
    assert_eq!(pick(vec![filter("ok", Eq, Some("false"))]).await, [1, 3, 5]);
    // Dates, from a date or a date-time.
    assert_eq!(
        pick(vec![filter("at", Gt, Some("2024-01-03"))]).await,
        [3, 4, 5]
    );
    assert_eq!(
        pick(vec![filter("at", Lt, Some("2024-01-03T00:00:00Z"))]).await,
        [1]
    );
    assert_eq!(
        pick(vec![filter("at", Eq, Some("2024-01-03 00:00:00"))]).await,
        [2]
    );
    assert_eq!(
        pick(vec![
            filter("at", Gte, Some("2024-01-02")),
            filter("at", Lt, Some("2024-01-05"))
        ])
        .await,
        [1, 2, 3]
    );
    // UUIDs.
    assert_eq!(
        pick(vec![filter(
            "u",
            Eq,
            Some("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a04")
        )])
        .await,
        [4]
    );
    // Text patterns on dates and ObjectIds compare their text.
    assert_eq!(
        pick(vec![filter("at", Contains, Some("2024-01-04"))]).await,
        [3]
    );
    assert_eq!(pick(vec![filter("_id", EndsWith, Some("02"))]).await, [2]);
    // Text that does not parse as the column's type is compared as a string (matches nothing).
    assert_eq!(
        pick(vec![filter("n", Eq, Some("not a number"))]).await,
        Vec::<i64>::new()
    );
    assert_eq!(
        pick(vec![filter("at", Eq, Some("yesterday"))]).await,
        Vec::<i64>::new()
    );
    assert_eq!(pick(vec![filter("tag", Eq, Some("tag-2"))]).await, [2]);
}
each_engine!(filters_on_typed_fields, mongo);

async fn raw_filter_documents(c: &Ctx) {
    let n = 100;
    let t = make_items(c, "raw", n).await;
    let ids_for = |raw: &str| {
        let mut req = c.browse(&t);
        req.raw_where = Some(raw.to_owned());
        async move { (c.page_ids(&req, 0, 1000).await, req) }
    };
    // Relaxed JSON: unquoted keys, single quotes, trailing commas, comments, regex literals.
    assert_eq!(ids_for("{ grp: 3 }").await.0, expect(n, |i| i % 7 == 3));
    assert_eq!(
        ids_for("{ 'grp': { $gt: 4, }, }").await.0,
        expect(n, |i| i % 7 > 4)
    );
    assert_eq!(
        ids_for("{ $or: [ { grp: 1 }, { grp: 2 } ] }").await.0,
        expect(n, |i| i % 7 == 1 || i % 7 == 2)
    );
    assert_eq!(ids_for("{ _id: { $in: [1, 2, 3] } }").await.0, [1, 2, 3]);
    assert_eq!(
        ids_for("{ label: { $exists: false } }").await.0,
        expect(n, |i| i % 10 == 5)
    );
    assert_eq!(
        ids_for("{ label: null }").await.0,
        expect(n, |i| label_of(i).is_none())
    );
    assert_eq!(ids_for("{ name: /00007$/ }").await.0, [7]);
    assert_eq!(
        ids_for("{ name: /ITEM 0001/i }").await.0,
        (10..=19).collect::<Vec<_>>()
    );
    assert_eq!(
        ids_for("{ $expr: { $gt: ['$grp', 5] } }").await.0,
        expect(n, |i| i % 7 > 5)
    );
    assert_eq!(
        ids_for("{ grp: 3 } // only grp 3").await.0,
        expect(n, |i| i % 7 == 3)
    );
    assert_eq!(ids_for("{}").await.0.len(), n as usize);
    assert_eq!(
        ids_for("   ").await.0.len(),
        n as usize,
        "blank is no filter"
    );
    // Combined with the filter bar by AND, and counted.
    let (_, mut req) = ids_for("{ grp: { $gte: 3 } }").await;
    req.filters = vec![filter("_id", FilterOp::Lt, Some("20"))];
    assert_eq!(c.page_ids(&req, 0, 1000).await, expect(19, |i| i % 7 >= 3));
    assert_eq!(
        c.driver.count_rows(&req).await.unwrap().exact,
        Some(expect(19, |i| i % 7 >= 3).len() as u64)
    );
    // Broken filters are InvalidInput, not a hang or a panic.
    for bad in [
        "{ grp: ",
        "not json",
        "[1, 2]",
        "{ $where: }",
        "{ grp: ObjectId('xyz') }",
    ] {
        let mut req = c.browse(&t);
        req.raw_where = Some(bad.to_owned());
        let err = c.driver.browse_page(&req, 0, 10).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{bad:?}: {err:?}");
        assert!(c.driver.count_rows(&req).await.is_err());
    }
    // An operator the server rejects comes back as its error.
    let mut req = c.browse(&t);
    req.raw_where = Some("{ grp: { $nope: 1 } }".into());
    let err = c.driver.browse_page(&req, 0, 10).await.unwrap_err();
    assert!(matches!(&err, Error::Database { .. }), "{err:?}");
    assert_eq!(c.page_ids(&c.browse(&t), 0, 3).await, [1, 2, 3]);
}
each_engine!(raw_filter_documents, mongo);

async fn big_collection_is_quick(c: &Ctx) {
    let req = c.browse("big_collection");
    let started = Instant::now();
    let count = c.driver.count_rows(&req).await.unwrap();
    assert_eq!((count.exact, count.estimate), (None, Some(1_000_000)));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let (page, took) = c.fastest_page(&req, 0, 100).await;
    assert!(took < Duration::from_secs(1), "first page took {took:?}");
    assert_eq!(page.rows.len(), 100);
    assert_eq!(page.rows[0][0], int(1));
    let started = Instant::now();
    let page = c.page(&req, 500_000, 100).await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "page at 500000 took {:?}",
        started.elapsed()
    );
    assert_eq!(page.rows[0][0], int(500_001));
    let mut desc = req.clone();
    desc.sort = vec![sort("_id", true)];
    assert_eq!(c.page_ids(&desc, 0, 3).await, [1_000_000, 999_999, 999_998]);
    // A selective filter is counted exactly.
    let mut filtered = req.clone();
    filtered.filters = vec![filter("_id", FilterOp::Gt, Some("999990"))];
    assert_eq!(
        c.driver.count_rows(&filtered).await.unwrap().exact,
        Some(10)
    );
    // Sorting on an indexed date column.
    let mut by_date = req.clone();
    by_date.sort = vec![sort("created_at", true)];
    let started = Instant::now();
    assert_eq!(c.page_ids(&by_date, 0, 2).await, [1_000_000, 999_999]);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}
each_engine!(big_collection_is_quick, mongo);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => {
            (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits()
        }
        _ => a == b,
    }
}

async fn every_bson_type_reads_back_typed(c: &Ctx) {
    let name = c.scratch("types");
    let coll = c.mongo_coll(&name);
    let oid = ObjectId::parse_str("65f000000000000000000001").unwrap();
    let uuid4 = bson::Binary {
        subtype: bson::spec::BinarySubtype::Uuid,
        bytes: (0u8..16).collect(),
    };
    let uuid3 = bson::Binary {
        subtype: bson::spec::BinarySubtype::UuidOld,
        bytes: (16u8..32).collect(),
    };
    let dec = |s: &str| s.parse::<bson::Decimal128>().unwrap();
    coll.insert_one(doc! {
        "_id": 1,
        "i32": 42, "i32_min": i32::MIN, "i64": i64::MAX, "i64_min": i64::MIN, "i64_big": 9_007_199_254_740_993i64, "i64_small": 5i64,
        "dbl": 1.5, "nan": f64::NAN, "inf": f64::INFINITY, "ninf": f64::NEG_INFINITY, "negzero": -0.0f64, "dmax": f64::MAX, "dmin": 5e-324f64, "whole": 2.0f64,
        "dec": dec("1234567890.123456789012345678901234"), "dec_neg": dec("-0.000000000000000000000000000000001"), "dec_nan": dec("NaN"), "dec_inf": dec("Infinity"),
        "t": true, "f": false,
        "s": "héllo wörld 🎉 日本語", "s_empty": "", "s_nul": "a\u{0}b", "s_crlf": "line1\r\nline2\ttab", "s_zwj": "👨‍👩‍👧‍👦 e\u{301} \u{200b}\u{feff}",
        "null": Bson::Null,
        "oid": oid,
        "date": bson::DateTime::from_millis(1_709_213_820_123),
        "date_before_1970": bson::DateTime::from_millis(-1),
        "date_year_1": bson::DateTime::from_millis(-62_135_596_800_000),
        "date_year_9999": bson::DateTime::from_millis(253_402_300_799_999),
        "date_year_10000": bson::DateTime::from_millis(253_402_300_800_000),
        "date_before_year_0": bson::DateTime::from_millis(-62_167_219_200_001),
        "bin": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![0xde, 0xad] },
        "bin_empty": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "bin_big": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![7; 5000] },
        "uuid4": uuid4, "uuid3": uuid3,
        "md5": bson::Binary { subtype: bson::spec::BinarySubtype::Md5, bytes: vec![1; 16] },
        "obj": { "a": 1, "z": { "deep": [true, Bson::Null, 1.5] }, "oid": oid, "when": bson::DateTime::from_millis(1_709_213_820_123), "dec": dec("1.5"), "big": 9_007_199_254_740_993i64, "u": "ü" },
        "arr": [1, "two", { "three": 3 }, [4], Bson::Null],
        "empty_obj": {}, "empty_arr": [],
        "re": bson::Regex { pattern: "^ab+c$".into(), options: "i".into() },
        "ts": bson::Timestamp { time: 1_709_213_820, increment: 1 },
        "minkey": Bson::MinKey, "maxkey": Bson::MaxKey,
        "js": Bson::JavaScriptCode("function () { return 1; }".into()),
    })
    .await
    .unwrap();

    let dec_text = |v: &str| Value::Decimal(dec(v).to_string());
    let expected: Vec<(&str, Value)> = vec![
        ("i32", int(42)),
        ("i32_min", int(i32::MIN as i64)),
        ("i64", int(i64::MAX)),
        ("i64_min", int(i64::MIN)),
        ("i64_big", int(9_007_199_254_740_993)),
        ("i64_small", int(5)),
        ("dbl", Value::Float(1.5)),
        ("nan", Value::Float(f64::NAN)),
        ("inf", Value::Float(f64::INFINITY)),
        ("ninf", Value::Float(f64::NEG_INFINITY)),
        ("negzero", Value::Float(-0.0)),
        ("dmax", Value::Float(f64::MAX)),
        ("dmin", Value::Float(5e-324)),
        ("whole", Value::Float(2.0)),
        ("dec", dec_text("1234567890.123456789012345678901234")),
        ("dec_neg", dec_text("-0.000000000000000000000000000000001")),
        ("dec_nan", dec_text("NaN")),
        ("dec_inf", dec_text("Infinity")),
        ("t", Value::Bool(true)),
        ("f", Value::Bool(false)),
        ("s", text("héllo wörld 🎉 日本語")),
        ("s_empty", text("")),
        ("s_nul", text("a\u{0}b")),
        ("s_crlf", text("line1\r\nline2\ttab")),
        ("s_zwj", text("👨‍👩‍👧‍👦 e\u{301} \u{200b}\u{feff}")),
        ("null", Value::Null),
        ("oid", Value::ObjectId("65f000000000000000000001".into())),
        ("date", Value::DateTime("2024-02-29T13:37:00.123Z".into())),
        (
            "date_before_1970",
            Value::DateTime("1969-12-31T23:59:59.999Z".into()),
        ),
        (
            "date_year_1",
            Value::DateTime("0001-01-01T00:00:00.000Z".into()),
        ),
        (
            "date_year_9999",
            Value::DateTime("9999-12-31T23:59:59.999Z".into()),
        ),
        (
            "date_year_10000",
            Value::Other {
                type_name: "date".into(),
                text: "253402300800000".into(),
            },
        ),
        (
            "date_before_year_0",
            Value::Other {
                type_name: "date".into(),
                text: "-62167219200001".into(),
            },
        ),
        ("bin", Value::bytes(vec![0xde, 0xad])),
        ("bin_empty", Value::bytes(vec![])),
        (
            "uuid4",
            Value::Uuid(uuid::Uuid::from_bytes(std::array::from_fn(|i| i as u8)).to_string()),
        ),
        (
            "uuid3",
            Value::Uuid(uuid::Uuid::from_bytes(std::array::from_fn(|i| 16 + i as u8)).to_string()),
        ),
        ("md5", Value::bytes(vec![1; 16])),
        ("empty_obj", Value::Json(serde_json::json!({}))),
        ("empty_arr", Value::Json(serde_json::json!([]))),
        (
            "re",
            Value::Other {
                type_name: "regex".into(),
                text: "/^ab+c$/i".into(),
            },
        ),
        (
            "ts",
            Value::Other {
                type_name: "timestamp".into(),
                text: "Timestamp(1709213820, 1)".into(),
            },
        ),
        (
            "minkey",
            Value::Other {
                type_name: "minKey".into(),
                text: "MinKey".into(),
            },
        ),
        (
            "maxkey",
            Value::Other {
                type_name: "maxKey".into(),
                text: "MaxKey".into(),
            },
        ),
        (
            "js",
            Value::Other {
                type_name: "javascript".into(),
                text: "function () { return 1; }".into(),
            },
        ),
        ("never_set", missing()),
    ];
    let page = c.page(&c.browse(&name), 0, 10).await;
    let col = |n: &str| {
        page.columns
            .iter()
            .position(|c| c.name == n)
            .unwrap_or_else(|| panic!("column {n}"))
    };
    let all = c.all_rows(&c.browse(&name)).await;
    let q = sets_of(&c.run(&format!("db.{name}.find({{}})")).await).remove(0);
    let qcol = |n: &str| {
        q.columns
            .iter()
            .position(|c| c.name == n)
            .unwrap_or_else(|| panic!("query column {n}"))
    };
    let mut problems = Vec::new();
    for (field, want) in &expected {
        if *field == "never_set" {
            continue;
        }
        for (path, got) in [
            ("browse_page", page.rows[0][col(field)].clone()),
            ("browse_all", all[0][col(field)].clone()),
            ("find", q.rows[0][qcol(field)].clone()),
        ] {
            if !same(&got, want) {
                problems.push(format!(
                    "{field} via {path}: got {got:?}, expected {want:?}"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // Documents and arrays come back as relaxed Extended JSON.
    assert_eq!(
        page.rows[0][col("obj")],
        Value::Json(serde_json::json!({
            "a": 1,
            "z": { "deep": [true, null, 1.5] },
            "oid": { "$oid": "65f000000000000000000001" },
            "when": { "$date": "2024-02-29T13:37:00.123Z" },
            "dec": { "$numberDecimal": "1.5" },
            "big": 9_007_199_254_740_993_i64,
            "u": "ü",
        }))
    );
    assert_eq!(
        page.rows[0][col("arr")],
        Value::Json(serde_json::json!([1, "two", { "three": 3 }, [4], null]))
    );
    // Field order inside documents is kept.
    let Value::Json(obj) = &page.rows[0][col("obj")] else {
        unreachable!()
    };
    assert_eq!(
        obj.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["a", "z", "oid", "when", "dec", "big", "u"]
    );
    // Long binary is cut for the grid but complete for the export.
    assert!(
        matches!(&page.rows[0][col("bin_big")], Value::Bytes { data, len: 5000 } if data.len() == 4096)
    );
    assert!(
        matches!(&all[0][col("bin_big")], Value::Bytes { data, len: 5000 } if data.len() == 5000)
    );
    // Types per column.
    let family = |n: &str| page.columns[col(n)].family;
    assert_eq!(family("i32"), TypeFamily::Integer);
    assert_eq!(family("dec"), TypeFamily::Decimal);
    assert_eq!(family("oid"), TypeFamily::ObjectId);
    assert_eq!(family("uuid4"), TypeFamily::Uuid);
    assert_eq!(family("obj"), TypeFamily::Document);
    assert_eq!(family("arr"), TypeFamily::Array);
    assert_eq!(family("date"), TypeFamily::DateTime);
    assert_eq!(page.columns[col("i32")].type_name, "int");
    assert_eq!(page.columns[col("i64")].type_name, "long");
    assert_eq!(page.columns[col("dbl")].type_name, "double");
    assert_eq!(page.columns[col("s")].type_name, "string");
    assert_eq!(page.columns[col("bin")].type_name, "binData");
    assert_eq!(page.columns[col("uuid4")].type_name, "uuid");
}
each_engine!(every_bson_type_reads_back_typed, mongo);

async fn null_missing_and_empty_are_three_things(c: &Ctx) {
    let name = c.scratch("null_kinds");
    c.mongo_coll(&name)
        .insert_many([
            doc! { "_id": 1, "v": Bson::Null },
            doc! { "_id": 2, "v": "" },
            doc! { "_id": 3 },
            doc! { "_id": 4, "v": "NULL" },
            doc! { "_id": 5, "v": 0 },
        ])
        .await
        .unwrap();
    let rows = c.page(&c.browse(&name), 0, 10).await.rows;
    let vs: Vec<Value> = rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(
        vs,
        vec![Value::Null, text(""), missing(), text("NULL"), int(0)]
    );
    // Equality with an empty string does not match null or absent.
    let mut req = c.browse(&name);
    req.filters = vec![filter("v", FilterOp::Eq, Some(""))];
    assert_eq!(c.page_ids(&req, 0, 10).await, [2]);
}
each_engine!(null_missing_and_empty_are_three_things, mongo);

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

fn key(id: impl Into<Value>) -> Vec<ColumnValue> {
    vec![ColumnValue {
        column: "_id".into(),
        value: id.into(),
    }]
}

fn upd(key: Vec<ColumnValue>, original: &[(&str, Value)], set: &[(&str, CellInput)]) -> RowChange {
    RowChange::Update {
        key,
        original: original
            .iter()
            .map(|(c, v)| ColumnValue {
                column: (*c).into(),
                value: v.clone(),
            })
            .collect(),
        set: set
            .iter()
            .map(|(c, v)| ColumnInput {
                column: (*c).into(),
                value: v.clone(),
            })
            .collect(),
    }
}

fn ins(values: &[(&str, CellInput)]) -> RowChange {
    RowChange::Insert {
        values: values
            .iter()
            .map(|(c, v)| ColumnInput {
                column: (*c).into(),
                value: v.clone(),
            })
            .collect(),
    }
}

fn set_of(c: &Ctx, t: &str, changes: Vec<RowChange>) -> ChangeSet {
    ChangeSet {
        table: c.tref(t),
        changes,
    }
}

async fn raw_doc(c: &Ctx, t: &str, id: impl Into<Bson>) -> Option<Document> {
    c.mongo_coll(t)
        .find_one(doc! { "_id": id.into() })
        .await
        .unwrap()
}

async fn changes_preview_and_apply(c: &Ctx) {
    let t = make_items(c, "changes", 10).await;
    let set = set_of(
        c,
        &t,
        vec![
            ins(&[
                ("_id", txt("100")),
                ("name", txt("O'Brien \"q\" \\")),
                ("grp", txt("5")),
                ("label", CellInput::Null),
                ("extra", txt("new field")),
            ]),
            upd(
                key(Value::Int(1)),
                &[("name", text("item 00001"))],
                &[("name", txt("renamed")), ("grp", txt("6"))],
            ),
            upd(key(Value::Int(2)), &[], &[("label", CellInput::Default)]),
            RowChange::Delete {
                key: key(Value::Int(3)),
            },
        ],
    );
    let preview = c.driver.preview_changes(&set).await.unwrap();
    assert_eq!(preview.len(), 4);
    assert!(
        preview[0].starts_with(&format!("db.{t}.insertOne({{")),
        "{}",
        preview[0]
    );
    assert!(
        preview[0].contains("_id: 100")
            && preview[0].contains("grp: 5")
            && preview[0].contains("label: null"),
        "{}",
        preview[0]
    );
    assert!(
        preview[0].contains(r#""O'Brien \"q\" \\""#),
        "{}",
        preview[0]
    );
    assert!(
        preview[1].starts_with(&format!(
            "db.{t}.updateOne({{ _id: 1, name: \"item 00001\" }}, {{ $set: {{"
        )),
        "{}",
        preview[1]
    );
    assert!(
        preview[2].contains("$unset: { label: \"\" }"),
        "{}",
        preview[2]
    );
    assert_eq!(preview[3], format!("db.{t}.deleteOne({{ _id: 3 }})"));
    assert_eq!(c.side_count(&t).await, 10, "preview changes nothing");

    let applied = c.driver.apply_changes(&set).await.unwrap();
    assert_eq!((applied.statements, applied.rows_affected), (4, 4));
    // The types follow the column: grp stays a number, the new field is text.
    let inserted = raw_doc(c, &t, 100).await.unwrap();
    assert_eq!(inserted.get("grp"), Some(&Bson::Int32(5)));
    assert_eq!(inserted.get("label"), Some(&Bson::Null));
    assert_eq!(
        inserted.get("extra"),
        Some(&Bson::String("new field".into()))
    );
    assert_eq!(inserted.get_str("name").unwrap(), "O'Brien \"q\" \\");
    let updated = raw_doc(c, &t, 1).await.unwrap();
    assert_eq!(updated.get_str("name").unwrap(), "renamed");
    assert_eq!(updated.get("grp"), Some(&Bson::Int32(6)));
    assert!(
        !raw_doc(c, &t, 2).await.unwrap().contains_key("label"),
        "Default unsets the field"
    );
    assert!(raw_doc(c, &t, 3).await.is_none());
    assert_eq!(c.side_count(&t).await, 10);

    // A cell can hold JSON, bytes and typed dates.
    let set = set_of(
        c,
        &t,
        vec![upd(
            key(Value::Int(4)),
            &[],
            &[
                (
                    "doc",
                    CellInput::Json(
                        serde_json::json!({ "k": [1, { "x": null }], "when": { "$date": "2024-01-02T03:04:05.678Z" } }),
                    ),
                ),
                ("raw", CellInput::Bytes("3q2+7w==".into())),
                ("flag", CellInput::Json(serde_json::json!(true))),
                (
                    "oid",
                    CellInput::Json(serde_json::json!({ "$oid": "65f000000000000000000001" })),
                ),
            ],
        )],
    );
    c.driver.apply_changes(&set).await.unwrap();
    let d = raw_doc(c, &t, 4).await.unwrap();
    assert_eq!(
        d.get_document("doc").unwrap().get_array("k").unwrap().len(),
        2
    );
    assert!(d.get_document("doc").unwrap().get_datetime("when").is_ok());
    assert_eq!(
        d.get_binary_generic("raw").unwrap(),
        &vec![0xde, 0xad, 0xbe, 0xef]
    );
    assert!(d.get_bool("flag").unwrap());
    assert!(d.get_object_id("oid").is_ok());
    // Without an _id the server makes one.
    c.driver
        .apply_changes(&set_of(c, &t, vec![ins(&[("name", txt("auto id"))])]))
        .await
        .unwrap();
    let auto = c
        .mongo_coll(&t)
        .find_one(doc! { "name": "auto id" })
        .await
        .unwrap()
        .unwrap();
    assert!(auto.get_object_id("_id").is_ok());
}
each_engine!(changes_preview_and_apply, mongo);

async fn typed_text_edits_keep_the_bson_type(c: &Ctx) {
    let name = c.scratch("typed_edit");
    let oid = ObjectId::parse_str("65f000000000000000000001").unwrap();
    c.mongo_coll(&name)
        .insert_one(doc! {
            "_id": 1, "i32": 7, "i64": 7i64, "i64_big": 9_007_199_254_740_993i64, "dbl": 1.5, "whole": 2.0f64,
            "dec": "12.50".parse::<bson::Decimal128>().unwrap(), "t": true, "s": "text", "s_num": "123",
            "oid": oid, "at": bson::DateTime::from_millis(1_709_213_820_123),
            "u": bson::Binary { subtype: bson::spec::BinarySubtype::Uuid, bytes: vec![0xa0; 16] },
            "bin": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![0xde, 0xad] },
        })
        .await
        .unwrap();
    let before = raw_doc(c, &name, 1).await.unwrap();
    // Typing back what is shown changes nothing about the stored types.
    let page = c.page(&c.browse(&name), 0, 1).await;
    let row = &page.rows[0];
    let shown = |n: &str| row[page.columns.iter().position(|c| c.name == n).unwrap()].clone();
    for field in [
        "i32", "i64", "i64_big", "dbl", "whole", "dec", "t", "s", "s_num", "oid", "at", "u", "bin",
    ] {
        let loaded = shown(field);
        let set = set_of(
            c,
            &name,
            vec![upd(
                key(Value::Int(1)),
                &[(field, loaded.clone())],
                &[(field, CellInput::Text(loaded.to_display()))],
            )],
        );
        c.driver
            .apply_changes(&set)
            .await
            .unwrap_or_else(|e| panic!("{field}: {e:?}"));
        let after = raw_doc(c, &name, 1).await.unwrap();
        assert_eq!(
            after.get(field),
            before.get(field),
            "{field} keeps its BSON type and value"
        );
    }
    // New numbers keep the width of the field; a number too big for int32 widens it.
    c.driver
        .apply_changes(&set_of(
            c,
            &name,
            vec![upd(
                key(Value::Int(1)),
                &[],
                &[
                    ("i32", txt("8")),
                    ("i64", txt("8")),
                    ("dbl", txt("2")),
                    ("dec", txt("0.10")),
                    ("t", txt("false")),
                ],
            )],
        ))
        .await
        .unwrap();
    let d = raw_doc(c, &name, 1).await.unwrap();
    assert_eq!(d.get("i32"), Some(&Bson::Int32(8)));
    assert_eq!(d.get("i64"), Some(&Bson::Int64(8)));
    assert_eq!(d.get("dbl"), Some(&Bson::Double(2.0)));
    assert_eq!(
        d.get("dec"),
        Some(&Bson::Decimal128("0.10".parse().unwrap()))
    );
    assert_eq!(d.get("t"), Some(&Bson::Boolean(false)));
    c.driver
        .apply_changes(&set_of(
            c,
            &name,
            vec![upd(
                key(Value::Int(1)),
                &[],
                &[
                    ("i32", txt("5000000000")),
                    ("s", txt("7")),
                    ("at", txt("2025-01-02 03:04:05")),
                ],
            )],
        ))
        .await
        .unwrap();
    let d = raw_doc(c, &name, 1).await.unwrap();
    assert_eq!(d.get("i32"), Some(&Bson::Int64(5_000_000_000)));
    assert_eq!(
        d.get("s"),
        Some(&Bson::String("7".into())),
        "a text field stays text"
    );
    assert_eq!(
        d.get("at"),
        Some(&Bson::DateTime(bson::DateTime::from_millis(
            1_735_787_045_000
        )))
    );
    // A document or array field must stay one.
    let err = c
        .driver
        .apply_changes(&set_of(
            c,
            &name,
            vec![ins(&[
                ("_id", txt("2")),
                ("obj", CellInput::Json(serde_json::json!({ "a": 1 }))),
            ])],
        ))
        .await;
    err.unwrap();
    let bad = set_of(
        c,
        &name,
        vec![upd(
            key(Value::Int(2)),
            &[],
            &[("obj", txt("not json at all"))],
        )],
    );
    assert!(
        matches!(
            c.driver.apply_changes(&bad).await,
            Err(Error::InvalidInput(_))
        ),
        "text in a document field"
    );
    assert!(
        raw_doc(c, &name, 2)
            .await
            .unwrap()
            .get_document("obj")
            .is_ok()
    );
}
each_engine!(typed_text_edits_keep_the_bson_type, mongo);

async fn documents_are_found_by_every_kind_of_id(c: &Ctx) {
    let name = c.scratch("id_kinds");
    let coll = c.mongo_coll(&name);
    let uuid4 = bson::Binary {
        subtype: bson::spec::BinarySubtype::Uuid,
        bytes: (1u8..=16).collect(),
    };
    let uuid3 = bson::Binary {
        subtype: bson::spec::BinarySubtype::UuidOld,
        bytes: (21u8..=36).collect(),
    };
    let ids: Vec<Bson> = vec![
        Bson::Int32(1),
        Bson::Int64(9_007_199_254_740_993),
        Bson::Double(2.5),
        Bson::String("string-id".into()),
        Bson::String(String::new()),
        Bson::ObjectId(ObjectId::new()),
        Bson::DateTime(bson::DateTime::from_millis(1_709_213_820_123)),
        Bson::Boolean(true),
        Bson::Binary(uuid4),
        Bson::Binary(uuid3),
        Bson::Document(doc! { "tenant": "b", "n": 2 }),
        Bson::Document(doc! { "b": 1, "a": 2 }),
        Bson::Decimal128("7.50".parse().unwrap()),
    ];
    for (i, id) in ids.iter().enumerate() {
        coll.insert_one(doc! { "_id": id.clone(), "n": i as i32 })
            .await
            .unwrap();
    }
    let page = c.page(&c.browse(&name), 0, 100).await;
    assert_eq!(page.rows.len(), ids.len());
    // Update each one by its displayed key, then delete it.
    for row in &page.rows {
        let id = row[0].clone();
        let n = match &row[1] {
            Value::Int(n) => *n,
            other => panic!("{other:?}"),
        };
        let update = set_of(
            c,
            &name,
            vec![upd(
                key(id.clone()),
                &[("n", int(n))],
                &[("n", txt(&(n + 100).to_string()))],
            )],
        );
        c.driver
            .apply_changes(&update)
            .await
            .unwrap_or_else(|e| panic!("updating by {id:?}: {e:?}"));
    }
    let bumped = coll
        .count_documents(doc! { "n": { "$gte": 100 } })
        .await
        .unwrap();
    assert_eq!(
        bumped as usize,
        ids.len(),
        "every document was found by its key"
    );
    for row in &page.rows {
        let id = row[0].clone();
        let deleted = c
            .driver
            .apply_changes(&set_of(
                c,
                &name,
                vec![RowChange::Delete {
                    key: key(id.clone()),
                }],
            ))
            .await;
        deleted.unwrap_or_else(|e| panic!("deleting by {id:?}: {e:?}"));
    }
    assert_eq!(c.side_count(&name).await, 0);
}
each_engine!(documents_are_found_by_every_kind_of_id, mongo);

async fn stale_edits_conflict_and_failed_sets_say_what_ran(c: &Ctx) {
    let t = make_items(c, "conflict", 5).await;
    let coll = c.mongo_coll(&t);
    // Someone else edits row 1.
    coll.update_one(
        doc! { "_id": 1 },
        doc! { "$set": { "name": "changed elsewhere" } },
    )
    .await
    .unwrap();
    let stale = set_of(
        c,
        &t,
        vec![upd(
            key(Value::Int(1)),
            &[("name", text("item 00001"))],
            &[("name", txt("mine"))],
        )],
    );
    let err = c.driver.apply_changes(&stale).await.unwrap_err();
    match &err {
        Error::Database {
            message,
            statement: Some(s),
            ..
        } => {
            assert_eq!(message, "Document was changed or deleted by someone else");
            assert!(s.contains("updateOne") && s.contains("mine"), "{s}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        raw_doc(c, &t, 1).await.unwrap().get_str("name").unwrap(),
        "changed elsewhere"
    );
    // A field that was null and is now set; a field that was absent and is now set.
    coll.update_one(doc! { "_id": 2 }, doc! { "$set": { "label": "now set" } })
        .await
        .unwrap();
    assert!(
        c.driver
            .apply_changes(&set_of(
                c,
                &t,
                vec![upd(
                    key(Value::Int(2)),
                    &[("label", missing())],
                    &[("label", txt("x"))]
                )]
            ))
            .await
            .is_err()
    );
    coll.update_one(doc! { "_id": 5 }, doc! { "$unset": { "label": "" } })
        .await
        .unwrap();
    c.driver
        .apply_changes(&set_of(
            c,
            &t,
            vec![upd(
                key(Value::Int(5)),
                &[("label", missing())],
                &[("label", txt("filled"))],
            )],
        ))
        .await
        .unwrap();
    // Deleting what is already gone.
    coll.delete_one(doc! { "_id": 4 }).await.unwrap();
    assert!(
        c.driver
            .apply_changes(&set_of(
                c,
                &t,
                vec![RowChange::Delete {
                    key: key(Value::Int(4))
                }]
            ))
            .await
            .is_err()
    );

    // Without transactions a failing set keeps what it already did, and the message says so.
    let set = set_of(
        c,
        &t,
        vec![
            ins(&[("_id", txt("50")), ("name", txt("first"))]),
            ins(&[("_id", txt("51")), ("name", txt("second"))]),
            ins(&[("_id", txt("3")), ("name", txt("duplicate key"))]),
            ins(&[("_id", txt("52")), ("name", txt("never reached"))]),
        ],
    );
    let err = c.driver.apply_changes(&set).await.unwrap_err();
    match &err {
        Error::Database {
            message,
            code,
            statement,
        } => {
            assert_eq!(code.as_deref(), Some("11000"), "{err:?}");
            assert!(
                message.contains("2 earlier changes were already applied"),
                "{message}"
            );
            assert!(
                statement
                    .as_deref()
                    .is_some_and(|s| s.contains("duplicate key"))
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(raw_doc(c, &t, 50).await.is_some() && raw_doc(c, &t, 51).await.is_some());
    assert!(raw_doc(c, &t, 52).await.is_none());

    // The _id cannot be changed.
    let err = c
        .driver
        .apply_changes(&set_of(
            c,
            &t,
            vec![upd(key(Value::Int(3)), &[], &[("_id", txt("33"))])],
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "66"),
        "{err:?}"
    );
    // Bad change sets are refused up front.
    for bad in [
        set_of(c, &t, vec![RowChange::Delete { key: vec![] }]),
        set_of(
            c,
            &t,
            vec![ins(&[
                ("_id", txt("60")),
                ("o", CellInput::Json(serde_json::json!({ "$oid": "zz" }))),
            ])],
        ),
    ] {
        assert!(c.driver.preview_changes(&bad).await.is_err());
        assert!(c.driver.apply_changes(&bad).await.is_err());
    }
    // An update that changes nothing is dropped.
    let noop = set_of(c, &t, vec![upd(key(Value::Int(3)), &[], &[])]);
    assert_eq!(c.driver.apply_changes(&noop).await.unwrap().statements, 0);
}
each_engine!(stale_edits_conflict_and_failed_sets_say_what_ran, mongo);

async fn racing_writers_have_one_winner(c: &Ctx) {
    let t = make_items(c, "race", 3).await;
    let mk = |new: &str| {
        set_of(
            c,
            &t,
            vec![upd(
                key(Value::Int(1)),
                &[("name", text("item 00001"))],
                &[("name", txt(new))],
            )],
        )
    };
    let (a, b) = (mk("from a"), mk("from b"));
    let (ra, rb) = tokio::join!(c.driver.apply_changes(&a), c.driver.apply_changes(&b));
    assert_eq!(
        [ra.is_ok(), rb.is_ok()].iter().filter(|w| **w).count(),
        1,
        "{ra:?} / {rb:?}"
    );
    let big: Vec<RowChange> = (1000..2000)
        .map(|i| ins(&[("_id", txt(&i.to_string())), ("name", txt("bulk"))]))
        .collect();
    let started = Instant::now();
    let applied = c.driver.apply_changes(&set_of(c, &t, big)).await.unwrap();
    assert_eq!(applied.rows_affected, 1000);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(c.side_count(&t).await, 1003);
}
each_engine!(racing_writers_have_one_winner, mongo);

// ---------------------------------------------------------------------------
// Shell statements
// ---------------------------------------------------------------------------

async fn shell_reads(c: &Ctx) {
    let t = make_items(c, "shell", 50).await;
    // find with the usual chain.
    let sets = sets_of(
        &run_mongo(
            c,
            &format!("db.{t}.find({{ grp: 3 }}).sort({{ _id: -1 }}).skip(1).limit(3)"),
        )
        .await,
    );
    assert_eq!(ids(&sets[0].rows, 0), [38, 31, 24]);
    assert_eq!(sets[0].names(), ["_id", "name", "grp", "label"]);
    // Projection, findOne, find with options, trailing semicolon, comments, newlines.
    let sets = sets_of(
        &run_mongo(
            c,
            &format!("db.{t}.find({{}}, {{ name: 1, _id: 0 }}).limit(2);"),
        )
        .await,
    );
    assert_eq!(sets[0].names(), ["name"]);
    assert_eq!(
        sets[0].rows,
        vec![vec![text("item 00001")], vec![text("item 00002")]]
    );
    let sets = sets_of(&run_mongo(c, &format!("db.{t}.findOne({{ _id: 7 }})")).await);
    assert_eq!(sets[0].rows.len(), 1);
    assert_eq!(sets[0].rows[0][0], int(7));
    let sets = sets_of(&run_mongo(c, &format!("// pick a few\ndb.{t}\n  .find({{ _id: {{ $lte: 3 }} }})\n  .sort({{ _id: 1 }}) /* inline */\n  .limit(10)")).await);
    assert_eq!(ids(&sets[0].rows, 0), [1, 2, 3]);
    let sets = sets_of(
        &run_mongo(
            c,
            &format!("db.{t}.find({{ name: /ITEM 0004/i }}).sort({{ _id: 1 }})"),
        )
        .await,
    );
    assert_eq!(sets[0].rows.len(), 10);
    // An empty result has no rows but still a (possibly empty) column list and no error.
    let events = run_mongo(c, &format!("db.{t}.find({{ _id: -1 }})")).await;
    assert!(sets_of(&events).iter().all(|s| s.rows.is_empty()));

    // Aggregation.
    let sets = sets_of(&run_mongo(c, &format!(
        "db.{t}.aggregate([ {{ $match: {{ _id: {{ $lte: 14 }} }} }}, {{ $group: {{ _id: \"$grp\", n: {{ $sum: 1 }} }} }}, {{ $sort: {{ _id: 1 }} }} ])"
    )).await);
    assert_eq!(sets[0].names(), ["_id", "n"]);
    assert_eq!(sets[0].rows.len(), 7);
    assert_eq!(sets[0].rows[0], vec![int(0), int(2)]);
    let sets = sets_of(
        &run_mongo(
            c,
            &format!("db.{t}.aggregate([ {{ $count: \"total\" }} ], {{ allowDiskUse: true }})"),
        )
        .await,
    );
    assert_eq!(sets[0].rows, vec![vec![int(50)]]);

    // Counts and distinct.
    assert_eq!(
        sets_of(&run_mongo(c, &format!("db.{t}.countDocuments({{ grp: 1 }})")).await)[0].rows,
        vec![vec![int(8)]]
    );
    assert_eq!(
        sets_of(&run_mongo(c, &format!("db.{t}.estimatedDocumentCount()")).await)[0].rows,
        vec![vec![int(50)]]
    );
    let distinct = sets_of(&run_mongo(c, &format!("db.{t}.distinct(\"grp\")")).await);
    assert_eq!(distinct[0].rows.len(), 7);

    // Commands and listings.
    let cmd = sets_of(&run_mongo(c, &format!("db.runCommand({{ count: \"{t}\" }})")).await);
    assert_eq!(
        cmd[0].rows[0][cmd[0].names().iter().position(|n| *n == "n").unwrap()],
        int(50)
    );
    let show = sets_of(&run_mongo(c, "show collections").await);
    assert!(show[0].rows.iter().any(|r| r[0] == text(&t)));
    let dbs = sets_of(&run_mongo(c, "show dbs").await);
    assert!(dbs[0].rows.iter().any(|r| r[0] == text(DB)));
    let names = sets_of(&run_mongo(c, "db.getCollectionNames()").await);
    assert!(names[0].rows.iter().any(|r| r[0] == text(&t)));

    // Awkward collection names.
    let events = run_mongo(c, "db.getCollection(\"with space\").find({})").await;
    assert_eq!(sets_of(&events)[0].names(), ["_id", "field with space"]);
    let events = run_mongo(c, "db.dots.in.name.find({})").await;
    assert_eq!(sets_of(&events)[0].rows.len(), 1);
    let events = run_mongo(
        c,
        "db.getSiblingDB(\"tablesail\").getCollection(\"select\").find({})",
    )
    .await;
    assert_eq!(sets_of(&events)[0].rows.len(), 1);
    let events = c
        .driver
        .query(Some("admin"), "db.runCommand({ ping: 1 })")
        .await
        .unwrap();
    assert!(!drain(events).await.unwrap().is_empty());
    // Documents are listed with their nested content.
    let sets = sets_of(&run_mongo(c, "db.deep_nesting.find({})").await);
    let Value::Json(child) =
        &sets[0].rows[0][sets[0].names().iter().position(|n| *n == "child").unwrap()]
    else {
        panic!()
    };
    assert_eq!(child["child"]["level"], 2);
}
each_engine!(shell_reads, mongo);

async fn shell_writes(c: &Ctx) {
    let t = c.scratch("shell_w");
    let affected = |events: &[CursorEvent]| affected_of(events);
    assert_eq!(
        affected(&run_mongo(c, &format!("db.{t}.insertOne({{ _id: 1, a: 1 }})")).await),
        [1]
    );
    assert_eq!(affected(&run_mongo(c, &format!("db.{t}.insertMany([ {{ _id: 2, a: 1 }}, {{ _id: 3, a: 2 }}, {{ _id: 4, a: 2 }} ])")).await), [3]);
    assert_eq!(
        affected(
            &run_mongo(
                c,
                &format!("db.{t}.updateOne({{ _id: 1 }}, {{ $set: {{ b: \"x\" }} }})")
            )
            .await
        ),
        [1]
    );
    assert_eq!(
        affected(
            &run_mongo(
                c,
                &format!("db.{t}.updateMany({{ a: 2 }}, {{ $set: {{ b: \"y\" }} }})")
            )
            .await
        ),
        [2]
    );
    assert_eq!(
        affected(
            &run_mongo(
                c,
                &format!("db.{t}.updateMany({{ a: 99 }}, {{ $set: {{ b: \"z\" }} }})")
            )
            .await
        ),
        [0]
    );
    assert_eq!(affected(&run_mongo(c, &format!("db.{t}.updateOne({{ _id: 9 }}, {{ $set: {{ b: \"up\" }} }}, {{ upsert: true }})")).await), [1], "an upsert counts");
    assert_eq!(
        affected(
            &run_mongo(
                c,
                &format!("db.{t}.replaceOne({{ _id: 9 }}, {{ replaced: true }})")
            )
            .await
        ),
        [1]
    );
    assert_eq!(affected(&run_mongo(c, &format!("db.{t}.updateOne({{ _id: 2 }}, [ {{ $set: {{ total: {{ $add: [\"$a\", 10] }} }} }} ])")).await), [1], "pipeline updates");
    assert_eq!(
        c.mongo_coll(&t)
            .find_one(doc! { "_id": 2 })
            .await
            .unwrap()
            .unwrap()
            .get_i32("total")
            .unwrap(),
        11
    );
    assert_eq!(
        affected(&run_mongo(c, &format!("db.{t}.deleteOne({{ _id: 1 }})")).await),
        [1]
    );
    assert_eq!(
        affected(&run_mongo(c, &format!("db.{t}.deleteMany({{ a: 2 }})")).await),
        [2]
    );
    assert_eq!(
        affected(&run_mongo(c, &format!("db.{t}.deleteMany({{ a: 2 }})")).await),
        [0]
    );
    assert_eq!(c.side_count(&t).await, 2);
    // Typed constructors in documents.
    run_mongo(c, &format!(
        "db.{t}.insertOne({{ _id: ObjectId(\"65f000000000000000000009\"), at: ISODate(\"2024-02-29T13:37:00.123Z\"), n: NumberLong(\"9007199254740993\"), d: NumberDecimal(\"1.50\"), u: UUID(\"a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11\"), re: /x+/i }})"
    ))
    .await;
    let d = c
        .mongo_coll(&t)
        .find_one(doc! { "_id": ObjectId::parse_str("65f000000000000000000009").unwrap() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        d.get_datetime("at").unwrap().timestamp_millis(),
        1_709_213_820_123
    );
    assert_eq!(d.get_i64("n").unwrap(), 9_007_199_254_740_993);
    assert!(matches!(d.get("d"), Some(Bson::Decimal128(_))));
    assert!(
        matches!(d.get("u"), Some(Bson::Binary(b)) if b.subtype == bson::spec::BinarySubtype::Uuid)
    );
    assert!(matches!(d.get("re"), Some(Bson::RegularExpression(_))));

    // Duplicate keys: the server's error and code, and the unordered rest still goes in.
    c.mongo_coll(&t)
        .insert_one(doc! { "_id": 3 })
        .await
        .unwrap();
    let err = c
        .events(&format!("db.{t}.insertOne({{ _id: 3 }})"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "11000"),
        "{err:?}"
    );
    let err = c
        .events(&format!("db.{t}.insertMany([ {{ _id: 3 }}, {{ _id: 30 }}, {{ _id: 31 }} ], {{ ordered: false }})"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(code), .. } if code == "11000"),
        "{err:?}"
    );
    assert!(
        c.mongo_coll(&t)
            .find_one(doc! { "_id": 30 })
            .await
            .unwrap()
            .is_some(),
        "unordered inserts continue past the failure"
    );
    let err = c
        .events(&format!(
            "db.{t}.insertMany([ {{ _id: 40 }}, {{ _id: 3 }}, {{ _id: 41 }} ])"
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Database { .. }));
    assert!(
        c.mongo_coll(&t)
            .find_one(doc! { "_id": 41 })
            .await
            .unwrap()
            .is_none(),
        "ordered inserts stop at the failure"
    );

    // Indexes and collections.
    let events = run_mongo(
        c,
        &format!("db.{t}.createIndex({{ a: 1, b: -1 }}, {{ unique: false, name: \"a_b\" }})"),
    )
    .await;
    assert!(!sets_of(&events).is_empty());
    let idx = sets_of(&run_mongo(c, &format!("db.{t}.getIndexes()")).await);
    assert!(
        idx[0]
            .rows
            .iter()
            .any(|r| r[idx[0].names().iter().position(|n| *n == "name").unwrap()] == text("a_b"))
    );
    run_mongo(c, &format!("db.{t}.dropIndex(\"a_b\")")).await;
    let idx = sets_of(&run_mongo(c, &format!("db.{t}.getIndexes()")).await);
    assert_eq!(idx[0].rows.len(), 1);
    let created = c.scratch("shell_created");
    run_mongo(
        c,
        &format!("db.createCollection(\"{created}\", {{ capped: true, size: 4096 }})"),
    )
    .await;
    assert!(
        c.driver
            .list_tables(DB, None)
            .await
            .unwrap()
            .iter()
            .any(|i| i.table.name == created)
    );
    let dropped = sets_of(&run_mongo(c, &format!("db.{created}.drop()")).await);
    assert_eq!(dropped[0].rows.len(), 1);
    assert!(
        !c.driver
            .list_tables(DB, None)
            .await
            .unwrap()
            .iter()
            .any(|i| i.table.name == created)
    );

    // The structure cache notices the schema change made through the shell.
    let before = c.driver.table_structure(&c.tref(&t)).await.unwrap();
    assert!(!before.columns.iter().any(|c| c.name == "fresh"));
    run_mongo(c, &format!("db.{t}.insertOne({{ _id: 500, fresh: 1 }})")).await;
    run_mongo(c, &format!("db.{t}.createIndex({{ fresh: 1 }})")).await;
    let after = c.driver.table_structure(&c.tref(&t)).await.unwrap();
    assert!(after.columns.iter().any(|c| c.name == "fresh"));
    assert!(after.indexes.iter().any(|i| i.columns == ["fresh"]));
}
each_engine!(shell_writes, mongo);

async fn shell_errors_are_clear(c: &Ctx) {
    for (statement, fragment) in [
        ("db.x.find(", "Unbalanced"),
        ("db.x.frobnicate({})", ""),
        ("db.x.find({ a: })", ""),
        ("", "Empty"),
        ("   ", "Empty"),
        ("use somewhere", ""),
        ("db.x.insertOne()", ""),
        ("db.x.find({ a: 'unterminated })", "Unterminated"),
        ("select * from x", ""),
    ] {
        match c.driver.query(None, statement).await {
            Err(Error::InvalidInput(message) | Error::Unsupported(message)) => {
                assert!(message.contains(fragment), "{statement:?}: {message}")
            }
            Err(other) => panic!("{statement:?}: {other:?}"),
            Ok(cursor) => {
                let result = drain(cursor).await;
                assert!(result.is_err(), "{statement:?} should fail, got {result:?}");
            }
        }
    }
    // Server-side errors keep their code.
    let err = c.events("db.runCommand({ nope: 1 })").await.unwrap_err();
    assert!(
        matches!(&err, Error::Database { code: Some(_), .. }),
        "{err:?}"
    );
    let err = c
        .events("db.big_collection.find({ $nope: 1 })")
        .await
        .unwrap_err();
    assert!(matches!(&err, Error::Database { .. }), "{err:?}");
    // The driver is fine afterwards.
    assert_eq!(
        sets_of(&run_mongo(c, "db.seed_done.countDocuments({})").await)[0].rows,
        vec![vec![int(1)]]
    );
}
each_engine!(shell_errors_are_clear, mongo);

async fn scripts_split_into_statements(c: &Ctx) {
    use dbcore::split::split_statements;
    let t = c.scratch("script");
    let script = format!(
        "db.{t}.insertOne({{ _id: 1, s: \"x;y\" }});
         db.{t}.insertMany([
             {{ _id: 2, s: 'multi' }},
             {{ _id: 3, s: \"it's; ok\" }}
         ])
         // a comment; with a semicolon
         db.{t}.find({{}})
           .sort({{ _id: 1 }})
           .limit(10)
         /* block; comment */ db.{t}.countDocuments({{}});
         db.{t}.updateOne({{ _id: 1 }}, {{ $set: {{ s: \"a\\\"b;c\" }} }})"
    );
    let spans = split_statements(&script, c.dialect);
    assert_eq!(
        spans.len(),
        5,
        "{:#?}",
        spans.iter().map(|s| &s.sql).collect::<Vec<_>>()
    );
    for span in &spans {
        assert_eq!(&script[span.start..span.end], span.sql);
    }
    let results = run_script(c, &script).await;
    assert_eq!(results.len(), 5);
    for (sql, result) in &results {
        result.as_ref().unwrap_or_else(|e| panic!("{e:?}\n{sql}"));
    }
    assert_eq!(affected_of(results[1].1.as_ref().unwrap()), [2]);
    assert_eq!(
        ids(&sets_of(results[2].1.as_ref().unwrap())[0].rows, 0),
        [1, 2, 3]
    );
    assert_eq!(
        sets_of(results[3].1.as_ref().unwrap())[0].rows,
        vec![vec![int(3)]]
    );
    assert_eq!(
        c.mongo_coll(&t)
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .unwrap()
            .get_str("s")
            .unwrap(),
        "a\"b;c"
    );
}
each_engine!(scripts_split_into_statements, mongo);

// ---------------------------------------------------------------------------
// Weird names
// ---------------------------------------------------------------------------

async fn awkward_collection_and_field_names(c: &Ctx) {
    let name = format!("{}_my coll \"q\" 'x' ünï 🎉 select", c.prefix);
    c.cleanup_collection(&name);
    let coll = c.mongo_coll(&name);
    let fields = [
        "select",
        "order",
        "Mixed Case",
        "with space",
        "a\"b",
        "a'b",
        "semi;colon",
        "--dash",
        "100% _wild_",
        "ünï 🎉",
    ];
    for i in 1..=4 {
        let mut d = doc! { "_id": i };
        for f in fields {
            d.insert(f, format!("{f}-{}", 5 - i));
        }
        coll.insert_one(d).await.unwrap();
    }
    let tables = c.driver.list_tables(DB, None).await.unwrap();
    assert!(tables.iter().any(|t| t.table.name == name));
    let s = c.driver.table_structure(&c.tref(&name)).await.unwrap();
    assert_eq!(s.table.name, name);
    for f in fields {
        assert!(s.columns.iter().any(|c| c.name == f), "{f}");
    }
    let page = c.page(&c.browse(&name), 0, 10).await;
    assert_eq!(page.rows.len(), 4);
    assert!(page.query.contains("db.getCollection("), "{}", page.query);
    for f in fields {
        let mut req = c.browse(&name);
        req.sort = vec![sort(f, false)];
        assert_eq!(
            c.page_ids(&req, 0, 10).await,
            [4, 3, 2, 1],
            "sorted by `{f}`"
        );
        let mut req = c.browse(&name);
        req.filters = vec![filter(f, FilterOp::Eq, Some(&format!("{f}-3")))];
        assert_eq!(c.page_ids(&req, 0, 10).await, [2], "filter on `{f}`");
        req.filters = vec![filter(f, FilterOp::Contains, Some("-2"))];
        assert_eq!(c.page_ids(&req, 0, 10).await, [3], "contains on `{f}`");
        let mut req = c.browse(&name);
        req.columns = vec![f.to_owned()];
        let page = c.page(&req, 0, 1).await;
        assert_eq!(
            page.columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["_id", f]
        );
    }
    // Edits through a change set, with the shell text in the preview.
    let set = set_of(
        c,
        &name,
        vec![
            upd(
                key(Value::Int(2)),
                &[("with space", text("with space-3"))],
                &[("with space", txt("edited")), ("a\"b", txt("quote\"d"))],
            ),
            ins(&[
                ("_id", txt("9")),
                ("ünï 🎉", txt("new")),
                ("100% _wild_", txt("pct")),
            ]),
            RowChange::Delete {
                key: key(Value::Int(4)),
            },
        ],
    );
    let preview = c.driver.preview_changes(&set).await.unwrap();
    assert!(
        preview[0].starts_with("db.getCollection("),
        "{}",
        preview[0]
    );
    assert!(
        preview[0].contains(r#""with space""#) && preview[0].contains(r#""a\"b""#),
        "{}",
        preview[0]
    );
    c.driver.apply_changes(&set).await.unwrap();
    let d = raw_doc(c, &name, 2).await.unwrap();
    assert_eq!(d.get_str("with space").unwrap(), "edited");
    assert_eq!(d.get_str("a\"b").unwrap(), "quote\"d");
    assert!(raw_doc(c, &name, 4).await.is_none());
    assert_eq!(
        raw_doc(c, &name, 9)
            .await
            .unwrap()
            .get_str("ünï 🎉")
            .unwrap(),
        "new"
    );
    // The shell reaches it too.
    let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
    let events = run_mongo(
        c,
        &format!("db.getCollection(\"{quoted}\").find({{ \"with space\": \"edited\" }})"),
    )
    .await;
    assert_eq!(sets_of(&events)[0].rows.len(), 1);
    let events = run_mongo(
        c,
        &format!(
            "db.getCollection('{}').countDocuments({{}})",
            name.replace('\'', "\\'")
        ),
    )
    .await;
    assert_eq!(sets_of(&events)[0].rows, vec![vec![int(4)]]);
    // Exports carry the names.
    assert_eq!(c.all_rows(&c.browse(&name)).await.len(), 4);
}
each_engine!(awkward_collection_and_field_names, mongo);

// ---------------------------------------------------------------------------
// Cancelling and dropping cursors
// ---------------------------------------------------------------------------

async fn operations_on(c: &Ctx, collection: &str) -> usize {
    let reply = c
        .mongo_client()
        .database("admin")
        .run_command(doc! { "currentOp": 1, "active": true, "ns": format!("{DB}.{collection}") })
        .await
        .unwrap();
    reply.get_array("inprog").map_or(0, Vec::len)
}

async fn open_cursors(c: &Ctx) -> i64 {
    let status = c
        .mongo_client()
        .database("admin")
        .run_command(doc! { "serverStatus": 1 })
        .await
        .unwrap();
    let total = status
        .get_document("metrics")
        .unwrap()
        .get_document("cursor")
        .unwrap()
        .get_document("open")
        .unwrap();
    match total.get("total") {
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Int32(n)) => i64::from(*n),
        other => panic!("{other:?}"),
    }
}

fn cancel_after(handle: std::sync::Arc<dyn dbcore::driver::CancelHandle>, delay: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        handle.cancel().await.unwrap();
    });
}

async fn cancelling_a_server_side_sleep(c: &Ctx) {
    let t = c.scratch("cancel");
    c.mongo_coll(&t)
        .insert_one(doc! { "_id": 1 })
        .await
        .unwrap();
    let mut cursor = c
        .driver
        .query(
            None,
            &format!("db.{t}.find({{ $where: \"sleep(30000) || true\" }})"),
        )
        .await
        .unwrap();
    cancel_after(cursor.cancel_handle(), Duration::from_millis(500));
    let started = Instant::now();
    let outcome = loop {
        match cursor.next(10).await {
            Ok(Some(_)) => continue,
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    drop(cursor);
    eventually(
        Duration::from_secs(10),
        "the server stops running the script",
        || async { operations_on(c, &t).await == 0 },
    )
    .await;
    // Cancelling a finished statement is harmless, and the next one is not affected.
    let mut done = c
        .driver
        .query(None, &format!("db.{t}.find({{}})"))
        .await
        .unwrap();
    let stale = done.cancel_handle();
    while done.next(10).await.unwrap().is_some() {}
    drop(done);
    stale.cancel().await.unwrap();
    let mut next = c
        .driver
        .query(
            None,
            &format!("db.{t}.find({{ $where: \"sleep(500) || true\" }})"),
        )
        .await
        .unwrap();
    stale.cancel().await.unwrap();
    let mut rows = 0;
    while let Some(event) = next
        .next(10)
        .await
        .expect("a stale handle must not cancel a later statement")
    {
        if let CursorEvent::Rows(r) = event {
            rows += r.len();
        }
    }
    assert_eq!(rows, 1);
    assert_eq!(
        sets_of(&run_mongo(c, &format!("db.{t}.countDocuments({{}})")).await)[0].rows,
        vec![vec![int(1)]]
    );
}
each_engine!(cancelling_a_server_side_sleep, mongo);

async fn dropping_cursors_frees_the_server(c: &Ctx) {
    let baseline = open_cursors(c).await;
    for i in 0..40 {
        let mut cursor = c
            .driver
            .query(None, "db.big_collection.find({})")
            .await
            .unwrap();
        let _ = cursor.next(10).await.unwrap(); // columns
        if i % 2 == 0 {
            let _ = cursor.next(10).await.unwrap(); // some documents
        }
        drop(cursor);
    }
    eventually(
        Duration::from_secs(10),
        "abandoned server cursors are closed",
        || async { open_cursors(c).await <= baseline + 6 },
    )
    .await;
    // Browse cursors (the export stream) too.
    for _ in 0..20 {
        let mut cursor = c
            .driver
            .browse_all(&c.browse("big_collection"))
            .await
            .unwrap();
        let _ = cursor.next(10).await.unwrap();
        drop(cursor);
    }
    eventually(
        Duration::from_secs(10),
        "abandoned export cursors are closed",
        || async { open_cursors(c).await <= baseline + 6 },
    )
    .await;
    // Plenty more abandoned cursors than the client has connections, and it still answers.
    let started = Instant::now();
    assert_eq!(
        sets_of(&run_mongo(c, "db.seed_done.countDocuments({})").await)[0].rows,
        vec![vec![int(1)]]
    );
    assert!(started.elapsed() < Duration::from_secs(3));

    // More statements at once than the connection pool holds all finish.
    let t = c.scratch("parallel");
    c.mongo_coll(&t)
        .insert_one(doc! { "_id": 1 })
        .await
        .unwrap();
    let started = Instant::now();
    let mut jobs = Vec::new();
    for _ in 0..24 {
        let driver = c.driver.clone();
        let sql = format!("db.{t}.find({{ $where: \"sleep(200) || true\" }})");
        jobs.push(tokio::spawn(async move {
            let cursor = driver.query(None, &sql).await?;
            drain(cursor).await
        }));
    }
    for job in jobs {
        let events = job.await.unwrap().expect("every statement finishes");
        assert_eq!(sets_of(&events)[0].rows.len(), 1);
    }
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
}
each_engine!(dropping_cursors_frees_the_server, mongo);

// ---------------------------------------------------------------------------
// Query log
// ---------------------------------------------------------------------------

async fn the_log_records_what_was_sent(c: &Ctx) {
    use dbcore::querylog;
    let t = make_items(c, "log", 20).await;
    let seq = querylog::since(0).last().map_or(0, |e| e.seq);
    let mut req = c.browse(&t);
    req.filters = vec![filter("name", FilterOp::Contains, Some("00007"))];
    req.sort = vec![sort("grp", true)];
    let page = c.page(&req, 0, 25).await;
    let _ = c.driver.count_rows(&req).await.unwrap();
    let _ = c.all_rows(&req).await;
    let _ = c.driver.count_rows(&c.browse(&t)).await.unwrap();
    let marker = format!("{t}_shell");
    let statement = format!("db.{t}.find({{ name: \"item 00001\" }}) // {marker}");
    let _ = run_mongo(c, &statement).await;
    c.driver
        .apply_changes(&set_of(
            c,
            &t,
            vec![
                ins(&[("_id", txt("77")), ("name", txt("logged"))]),
                upd(key(Value::Int(1)), &[], &[("name", txt("logged update"))]),
                RowChange::Delete {
                    key: key(Value::Int(2)),
                },
            ],
        ))
        .await
        .unwrap();
    let mine: Vec<_> = querylog::since(seq)
        .into_iter()
        .filter(|e| e.sql.contains(&t))
        .collect();
    let sql: Vec<&str> = mine.iter().map(|e| e.sql.as_str()).collect();
    assert!(
        sql.contains(&page.query.as_str()),
        "the page's find is logged: {sql:#?}"
    );
    assert!(
        sql.iter()
            .any(|s| s.contains(".countDocuments(") && s.contains("00007")),
        "{sql:#?}"
    );
    assert!(
        sql.iter().any(|s| s.contains(".estimatedDocumentCount()")),
        "{sql:#?}"
    );
    assert!(
        sql.iter()
            .filter(|s| s.contains("find(") && s.contains("00007"))
            .count()
            >= 2,
        "page and export: {sql:#?}"
    );
    assert!(
        sql.iter()
            .any(|s| s.starts_with(&format!("db.{t}.find({{ name: \"item 00001\" }})"))),
        "typed statements are logged as typed: {sql:#?}"
    );
    let pos = |needle: &str| {
        sql.iter()
            .position(|s| s.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing in {sql:#?}"))
    };
    assert!(pos(".insertOne(") < pos(".updateOne(") && pos(".updateOne(") < pos(".deleteOne("));
}
each_engine!(the_log_records_what_was_sent, mongo);
