// The "nasty schema" for MongoDB: every BSON type, shapeless collections,
// awkward names and a 1M-document collection. Runs in mongosh against `tablesail`.

const tdb = db.getSiblingDB('tablesail');

// ---------------------------------------------------------------------------
// Every BSON type
// ---------------------------------------------------------------------------
tdb.all_types.insertMany([
  {
    _id: ObjectId('65f000000000000000000001'),
    string: 'hello',
    int32: NumberInt(42),
    int64: NumberLong('9007199254740993'),
    double: 2.718281828459045,
    decimal128: NumberDecimal('12345.678901234567890123456789012'),
    bool: true,
    date: ISODate('2024-02-29T13:37:00.123Z'),
    timestamp: Timestamp({ t: 1709213820, i: 1 }),
    binary: BinData(0, '3q2+7w=='),
    uuid: UUID('a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'),
    md5: BinData(5, '1B2M2Y8AsgTpgAmY7PhCfg=='),
    objectId: ObjectId('65f0000000000000000000ff'),
    regex: /^ab+c$/i,
    null: null,
    array: [1, 'two', { three: 3 }, [4]],
    emptyArray: [],
    object: { nested: { deep: { deeper: [true, null, 1.5] } } },
    emptyObject: {},
    minKey: MinKey(),
    maxKey: MaxKey(),
    code: Code('function () { return 1; }'),
  },
  {
    _id: ObjectId('65f000000000000000000002'),
    string: '',
    int32: NumberInt(-2147483648),
    int64: NumberLong('-9223372036854775808'),
    double: -Infinity,
    decimal128: NumberDecimal('-Infinity'),
    bool: false,
    date: new Date(-62135596800000), // 0001-01-01
    null: null,
  },
  {
    _id: ObjectId('65f000000000000000000003'),
    string: 'line one\nline two\ttabbed — emoji 🚀 — RTL مرحبا — zero-width​join',
    int32: NumberInt(2147483647),
    int64: NumberLong('9223372036854775807'),
    double: NaN,
    decimal128: NumberDecimal('NaN'),
    date: new Date(253402300799999), // 9999-12-31T23:59:59.999Z
    bigString: 'x'.repeat(1024 * 1024),
  },
]);

// ---------------------------------------------------------------------------
// Shapeless data: the same field with different types, missing fields, odd _ids
// ---------------------------------------------------------------------------
tdb.heterogeneous.insertMany([
  { _id: 1, value: 42, onlyHere: true },
  { _id: 'string-id', value: 'forty-two' },
  { _id: { compound: 'id', n: 1 }, value: [4, 2] },
  { _id: ObjectId(), value: { n: 42 } },
  { _id: UUID(), value: null },
  { _id: NumberLong(3) },
  { _id: 2.5, value: ISODate('2024-01-01T00:00:00Z'), extra: { a: 1 } },
]);

let deep = { level: 20 };
for (let i = 19; i >= 0; i--) deep = { level: i, child: deep };
tdb.deep_nesting.insertOne(deep);

// 500 distinct field names in one document.
const wide = {};
for (let i = 1; i <= 500; i++) wide[`field_${i}`] = i;
tdb.wide_document.insertOne(wide);

// ---------------------------------------------------------------------------
// Awkward names
// ---------------------------------------------------------------------------
tdb.getCollection('select').insertOne({ from: 1, where: 'w', order: 'o' });
tdb.getCollection('with space').insertOne({ 'field with space': 1 });
tdb.getCollection('dots.in.name').insertOne({ x: 1 });
tdb.getCollection('ñame 🔑').insertOne({ ñ: 'ñ', '🔑': '🗝' });
try {
  // MongoDB 5+ accepts dotted and $-prefixed field names; many clients mishandle them.
  tdb.getCollection('odd_fields').insertOne({ 'a.b': 'dotted', $dollar: 'dollar' });
} catch (e) {
  print(`odd_fields skipped: ${e}`);
}

// ---------------------------------------------------------------------------
// Relationships, views, special collections
// ---------------------------------------------------------------------------
tdb.customers.insertMany([
  { _id: 1, email: 'a@example.com', name: 'Alice' },
  { _id: 2, email: 'b@example.com', name: 'Bob' },
]);
tdb.orders.insertMany([
  { customer_id: 1, total: NumberDecimal('10.00'), items: [{ sku: 'A', qty: 1 }] },
  { customer_id: 1, total: NumberDecimal('20.00'), items: [] },
  { customer_id: 2, total: NumberDecimal('5.00') },
]);
tdb.orders.createIndex({ customer_id: 1 });

tdb.createView('customer_totals', 'orders', [
  { $group: { _id: '$customer_id', total: { $sum: '$total' } } },
]);

tdb.createCollection('capped_log', { capped: true, size: 1024 * 1024, max: 1000 });
tdb.capped_log.insertOne({ msg: 'capped' });

tdb.createCollection('metrics', { timeseries: { timeField: 'ts', metaField: 'sensor' } });
tdb.metrics.insertMany([
  { ts: ISODate('2024-01-01T00:00:00Z'), sensor: { id: 1 }, temp: 20.5 },
  { ts: ISODate('2024-01-01T00:01:00Z'), sensor: { id: 1 }, temp: 20.7 },
]);

tdb.createCollection('validated', {
  validator: { $jsonSchema: { bsonType: 'object', required: ['name'], properties: { name: { bsonType: 'string' } } } },
});
tdb.validated.insertOne({ name: 'valid' });

tdb.createCollection('empty_collection');

// ---------------------------------------------------------------------------
// 1M documents
// ---------------------------------------------------------------------------
const TOTAL = 1000000;
const BATCH = 10000;
const start = new Date('2020-01-01T00:00:00Z').getTime();
for (let base = 1; base <= TOTAL; base += BATCH) {
  const docs = [];
  for (let n = base; n < base + BATCH; n++) {
    const doc = {
      _id: NumberLong(n),
      name: `row ${n}`,
      amount: n % 10 === 0 ? null : NumberDecimal(((n % 100000) / 100).toFixed(2)),
      is_active: n % 2 === 0,
      created_at: new Date(start + n * 60000),
    };
    if (n % 3 === 0) doc.payload = { n: NumberInt(n), tags: ['a', 'b'] };
    docs.push(doc);
  }
  tdb.big_collection.insertMany(docs, { ordered: false });
}
tdb.big_collection.createIndex({ created_at: 1 });

// Written last: the compose healthcheck waits for this marker.
tdb.seed_done.insertOne({ at: new Date() });
