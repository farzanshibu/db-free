// SOT: ddl-end-to-end-tests, ddl-sqlite-roundtrip, ddl-duckdb-roundtrip

use super::{alter_table, assemble, create_table, duplicate_table, namespace, remove_table};
use crate::integrations::{duckdb, sqlite, Integration};
use crate::model::{
    AlterTableSpec, ColumnDefault, ColumnSpec, ConnectionInput, ConnectionSummary, DdlOperation, DuplicateSpec, Engine, FkAction, ForeignKeySpec,
    IndexSpec, NamespaceKind, NamespaceOp, NamespaceSpec, RemoveAction, RemoveSpec, ResolvedConnection, SshTunnel, StatementResult, TableKind,
    TableRef, TableSpec, Value,
};
use std::sync::Arc;

// WHAT:  Round trips against the two embedded engines: build a table from a spec,
//        run the statements, read it back with `describe`, edit it, run the diff,
//        read it back again.
// WHY:   The builders are pure string functions; only a real SQLite and a real
//        DuckDB can say the statements are accepted, that `describe` reads back
//        what was written, and that an untouched description produces no ALTER.
// HOW:   Each step runs the exact script `assemble` produces (BEGIN / COMMIT and
//        PRAGMAs included), the same text the form hands to `execute_query`.
// WHERE: src-tauri/src/integrations/ddl/mod.rs

type Db = Arc<dyn Integration>;

fn resolved(engine: Engine, path: &str) -> ResolvedConnection {
    let input: ConnectionInput = serde_json::from_value(serde_json::json!({
        "name": "t", "engine": engine, "environment": "local", "readOnly": false, "host": null, "port": null,
        "database": null, "username": null, "password": null, "filePath": path, "sslMode": "disable",
        "ssh": SshTunnel::default(), "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
    }))
    .unwrap_or_else(|e| panic!("{e}"));
    ResolvedConnection { summary: ConnectionSummary::draft(&input, false), secret: None }
}

async fn sqlite_db() -> (Db, String) {
    let dir = std::env::temp_dir().join(format!("db-free-ddl-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let path = dir.join("ddl.db").to_string_lossy().into_owned();
    let db = sqlite::connect(&resolved(Engine::Sqlite, &path)).await.unwrap_or_else(|e| panic!("{e}"));
    (db, path)
}

async fn duckdb_db() -> Db {
    duckdb::connect(&resolved(Engine::Duckdb, ":memory:")).await.unwrap_or_else(|e| panic!("{e}"))
}

async fn run(db: &Db, engine: Engine, operation: &DdlOperation, statements: Vec<String>) {
    let script = assemble(engine, operation, &statements).script;
    db.execute(&script, 100).await.unwrap_or_else(|e| panic!("{e}\n--- script ---\n{script}"));
}

async fn scalar(db: &Db, sql: &str) -> String {
    match db.execute(sql, 10).await.unwrap_or_else(|e| panic!("{e}\n{sql}")).first() {
        Some(StatementResult::Rows { result }) => match result.rows.first().and_then(|r| r.first()) {
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Text(s) | Value::Decimal(s) | Value::DateTime(s)) => s.clone(),
            Some(Value::Float(f)) => f.to_string(),
            Some(Value::Null) | None => "NULL".to_string(),
            Some(other) => format!("{other:?}"),
        },
        _ => "NONE".to_string(),
    }
}

fn t(name: &str) -> TableRef {
    TableRef { schema: None, name: name.into() }
}

fn column(name: &str, ty: &str) -> ColumnSpec {
    ColumnSpec {
        name: name.into(),
        original_name: None,
        data_type: ty.into(),
        length: None,
        precision: None,
        scale: None,
        wrapper: None,
        enum_values: Vec::new(),
        nullable: true,
        primary_key: false,
        unique: false,
        unique_name: None,
        auto_increment: false,
        unsigned: false,
        default: ColumnDefault::None,
    }
}

fn spec(name: &str, columns: Vec<ColumnSpec>) -> TableSpec {
    TableSpec { table: t(name), columns, primary_key_name: None, foreign_keys: Vec::new(), indexes: Vec::new(), engine: None, order_by: Vec::new(), if_not_exists: false }
}

async fn create(db: &Db, engine: Engine, table: &TableSpec) {
    let statements = create_table(engine, table).unwrap_or_else(|e| panic!("{e}"));
    run(db, engine, &DdlOperation::CreateTable { spec: table.clone() }, statements).await;
}

async fn alter(db: &Db, engine: Engine, original: &TableSpec, desired: &TableSpec) -> Vec<String> {
    let spec = AlterTableSpec { original: original.clone(), desired: desired.clone() };
    let statements = alter_table(engine, &spec).unwrap_or_else(|e| panic!("{e}"));
    run(db, engine, &DdlOperation::AlterTable { spec }, statements.clone()).await;
    statements
}

fn find<'a>(spec: &'a TableSpec, name: &str) -> &'a ColumnSpec {
    spec.columns.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no column {name} in {:?}", spec.columns.iter().map(|c| &c.name).collect::<Vec<_>>()))
}

async fn names(db: &Db) -> Vec<String> {
    let catalog = db.catalog().await.unwrap_or_else(|e| panic!("{e}"));
    let mut out: Vec<String> = catalog.schemas.iter().flat_map(|s| s.tables.iter().map(|t| t.name.clone())).collect();
    out.sort();
    out
}

#[tokio::test]
async fn sqlite_create_describe_alter_rebuild_duplicate_and_drop() {
    let engine = Engine::Sqlite;
    let (db, path) = sqlite_db().await;

    let mut owner_id = column("id", "INTEGER");
    owner_id.primary_key = true;
    owner_id.nullable = false;
    owner_id.auto_increment = true;
    let mut owner_name = column("name", "TEXT");
    owner_name.nullable = false;
    create(&db, engine, &spec("owners", vec![owner_id, owner_name])).await;

    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    id.nullable = false;
    id.auto_increment = true;
    let mut email = column("email", "TEXT");
    email.nullable = false;
    email.unique = true;
    let mut name = column("name", "VARCHAR");
    name.length = Some(40);
    name.default = ColumnDefault::Custom { expression: "'anon'".into() };
    let owner = column("owner_id", "INTEGER");
    let mut created = column("created", "DATETIME");
    created.default = ColumnDefault::Now;
    let mut people = spec("people", vec![id, email, name, owner, created]);
    people.foreign_keys = vec![ForeignKeySpec {
        name: None,
        original_name: None,
        columns: vec!["owner_id".into()],
        ref_table: t("owners"),
        ref_columns: vec!["id".into()],
        on_delete: FkAction::Cascade,
        on_update: FkAction::NoAction,
    }];
    people.indexes = vec![IndexSpec { name: "people_name_idx".into(), columns: vec!["name".into()], unique: false }];
    create(&db, engine, &people).await;
    db.execute("INSERT INTO owners (name) VALUES ('ann'), ('bob'); INSERT INTO people (email, owner_id) VALUES ('a@x', 1), ('b@x', 2)", 10).await.unwrap_or_else(|e| panic!("{e}"));

    // ---- describe reads back what was written ----
    let described = db.describe_table(&t("people")).await.unwrap_or_else(|e| panic!("{e}"));
    let id = find(&described, "id");
    assert!(id.primary_key && id.auto_increment && id.data_type == "INTEGER", "{id:?}");
    let email = find(&described, "email");
    assert!(email.unique && !email.nullable && email.unique_name.is_some(), "{email:?}");
    let name = find(&described, "name");
    assert_eq!((name.data_type.as_str(), name.length), ("VARCHAR", Some(40)));
    assert_eq!(name.default, ColumnDefault::Custom { expression: "'anon'".into() });
    assert_eq!(find(&described, "created").default, ColumnDefault::Now);
    assert_eq!(described.foreign_keys.len(), 1);
    assert_eq!((described.foreign_keys[0].on_delete, described.foreign_keys[0].ref_table.name.as_str()), (FkAction::Cascade, "owners"));
    assert_eq!(described.indexes, vec![IndexSpec { name: "people_name_idx".into(), columns: vec!["name".into()], unique: false }]);

    // An untouched description is a no-op: nothing to run.
    let noop = alter_table(engine, &AlterTableSpec { original: described.clone(), desired: described.clone() }).unwrap_or_else(|e| panic!("{e}"));
    assert!(noop.is_empty(), "{noop:#?}");

    // ---- in-place edits: add, rename, drop ----
    let mut desired = described.clone();
    let mut age = column("age", "INTEGER");
    age.nullable = false;
    age.default = ColumnDefault::Custom { expression: "0".into() };
    desired.columns.push(age);
    if let Some(c) = desired.columns.iter_mut().find(|c| c.name == "name") {
        c.name = "full_name".into();
    }
    desired.indexes[0].columns = vec!["full_name".into()];
    let statements = alter(&db, engine, &described, &desired).await;
    assert!(statements.iter().all(|s| !s.contains("__rebuild")), "a rename and an add are plain ALTERs: {statements:#?}");
    let after = db.describe_table(&t("people")).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(find(&after, "age").default, ColumnDefault::Custom { expression: "0".into() });
    assert_eq!(after.indexes[0].columns, vec!["full_name".to_string()]);
    assert_eq!(scalar(&db, "SELECT count(*) FROM people").await, "2");

    // ---- a change SQLite cannot ALTER: rebuild, keeping rows, FK and index ----
    let mut rebuild = after.clone();
    {
        let c = rebuild.columns.iter_mut().find(|c| c.name == "full_name").unwrap_or_else(|| panic!("full_name"));
        c.data_type = "TEXT".into();
        c.length = None;
        c.nullable = false;
        c.default = ColumnDefault::Empty;
    }
    if let Some(c) = rebuild.columns.iter_mut().find(|c| c.name == "owner_id") {
        c.nullable = false;
    }
    rebuild.foreign_keys[0].on_delete = FkAction::SetNull;
    let statements = alter(&db, engine, &after, &rebuild).await;
    assert!(statements.iter().any(|s| s.contains("__rebuild")), "{statements:#?}");
    let rebuilt = db.describe_table(&t("people")).await.unwrap_or_else(|e| panic!("{e}"));
    let full = find(&rebuilt, "full_name");
    assert_eq!((full.data_type.as_str(), full.nullable, full.default.clone()), ("TEXT", false, ColumnDefault::Empty));
    assert!(find(&rebuilt, "id").auto_increment && find(&rebuilt, "email").unique, "keys survive a rebuild");
    assert_eq!(rebuilt.foreign_keys[0].on_delete, FkAction::SetNull);
    assert_eq!(rebuilt.indexes.len(), 1);
    assert_eq!(scalar(&db, "SELECT count(*) FROM people").await, "2");
    assert_eq!(scalar(&db, "SELECT full_name FROM people WHERE id = 1").await, "anon", "NULLs were filled from the default");
    assert_eq!(scalar(&db, "SELECT count(*) FROM sqlite_master WHERE name LIKE '%__rebuild'").await, "0");
    // The auto-increment counter kept going.
    db.execute("INSERT INTO people (email, owner_id, full_name) VALUES ('c@x', 1, 'cy')", 10).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(scalar(&db, "SELECT max(id) FROM people").await, "3");

    // ---- rename the table, drop a column that is plain to drop ----
    let mut renamed = rebuilt.clone();
    renamed.table.name = "persons".into();
    renamed.columns.retain(|c| c.name != "age");
    alter(&db, engine, &rebuilt, &renamed).await;
    assert_eq!(names(&db).await, vec!["owners".to_string(), "persons".to_string()]);

    // ---- duplicate with rows and indexes ----
    let persons = db.describe_table(&t("persons")).await.unwrap_or_else(|e| panic!("{e}"));
    let copy = DuplicateSpec { source: t("persons"), target: t("persons_copy"), include_data: true, include_indexes: true };
    let statements = duplicate_table(engine, &copy, &persons).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::DuplicateTable { spec: copy }, statements).await;
    assert_eq!(scalar(&db, "SELECT count(*) FROM persons_copy").await, "3");
    let copied = db.describe_table(&t("persons_copy")).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(copied.indexes.len(), 1);
    assert!(copied.indexes[0].name.starts_with("persons_copy_"), "{:?}", copied.indexes);

    // ---- empty, then drop ----
    let remove = |action| RemoveSpec { table: t("persons_copy"), kind: TableKind::Table, action, cascade: false, if_exists: false };
    for action in [RemoveAction::Truncate, RemoveAction::Drop] {
        let spec = remove(action);
        let statements = remove_table(engine, &spec).unwrap_or_else(|e| panic!("{e}"));
        run(&db, engine, &DdlOperation::RemoveTable { spec }, statements).await;
        if action == RemoveAction::Truncate {
            assert_eq!(scalar(&db, "SELECT count(*) FROM persons_copy").await, "0");
        }
    }
    assert_eq!(names(&db).await, vec!["owners".to_string(), "persons".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn sqlite_refuses_nothing_the_namespace_forms_never_offered() {
    // SQLite is a single file: no schema or database can be created, renamed or dropped.
    let spec = NamespaceSpec { kind: NamespaceKind::Schema, name: "x".into(), op: NamespaceOp::Create { owner: None, if_not_exists: false } };
    assert!(namespace(Engine::Sqlite, &spec).is_err());
}

#[tokio::test]
async fn duckdb_create_describe_alter_duplicate_and_drop() {
    let engine = Engine::Duckdb;
    let db = duckdb_db().await;

    let mut owner_id = column("id", "INTEGER");
    owner_id.primary_key = true;
    owner_id.nullable = false;
    create(&db, engine, &spec("owners", vec![owner_id, column("name", "VARCHAR")])).await;

    let mut id = column("id", "INTEGER");
    id.primary_key = true;
    id.nullable = false;
    id.auto_increment = true;
    let mut email = column("email", "VARCHAR");
    email.nullable = false;
    email.unique = true;
    let mut score = column("score", "DECIMAL");
    score.precision = Some(10);
    score.scale = Some(2);
    score.default = ColumnDefault::Custom { expression: "0".into() };
    let mut created = column("created", "TIMESTAMP");
    created.default = ColumnDefault::Now;
    let mut people = spec("people", vec![id, email, score, column("owner_id", "INTEGER"), created]);
    people.foreign_keys = vec![ForeignKeySpec {
        name: Some("people_owner_fk".into()),
        original_name: None,
        columns: vec!["owner_id".into()],
        ref_table: t("owners"),
        ref_columns: vec!["id".into()],
        on_delete: FkAction::NoAction,
        on_update: FkAction::NoAction,
    }];
    create(&db, engine, &people).await;
    db.execute("INSERT INTO owners VALUES (1, 'ann'); INSERT INTO people (email, owner_id) VALUES ('a@x', 1), ('b@x', 1)", 10).await.unwrap_or_else(|e| panic!("{e}"));

    let described = db.describe_table(&t("people")).await.unwrap_or_else(|e| panic!("{e}"));
    let id = find(&described, "id");
    assert!(id.auto_increment && id.primary_key && id.default == ColumnDefault::None, "{id:?}");
    assert!(find(&described, "email").unique);
    let score = find(&described, "score");
    assert_eq!((score.data_type.as_str(), score.precision, score.scale), ("DECIMAL", Some(10), Some(2)));
    assert_eq!(find(&described, "created").default, ColumnDefault::Now);
    assert_eq!(described.foreign_keys.len(), 1);

    let noop = alter_table(engine, &AlterTableSpec { original: described.clone(), desired: described.clone() }).unwrap_or_else(|e| panic!("{e}"));
    assert!(noop.is_empty(), "{noop:#?}");

    // ---- ALTER in place: add (NOT NULL goes in two steps), rename, retype, default, drop ----
    let mut desired = described.clone();
    let mut note = column("note", "VARCHAR");
    note.nullable = false;
    note.default = ColumnDefault::Empty;
    desired.columns.push(note);
    if let Some(c) = desired.columns.iter_mut().find(|c| c.name == "score") {
        c.data_type = "DOUBLE".into();
        c.precision = None;
        c.scale = None;
        c.default = ColumnDefault::None;
    }
    desired.columns.retain(|c| c.name != "created");
    alter(&db, engine, &described, &desired).await;
    let after = db.describe_table(&t("people")).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(find(&after, "score").data_type, "DOUBLE");
    assert_eq!(find(&after, "score").default, ColumnDefault::None);
    assert!(!find(&after, "note").nullable);
    assert!(after.columns.iter().all(|c| c.name != "created"));
    assert_eq!(scalar(&db, "SELECT count(*) FROM people").await, "2");

    // ---- DuckDB refuses to rename a column a key depends on: said by name, nothing run ----
    let mut blocked = after.clone();
    if let Some(c) = blocked.columns.iter_mut().find(|c| c.name == "owner_id") {
        c.name = "owner".into();
    }
    blocked.foreign_keys[0].columns = vec!["owner".into()];
    let refused = alter_table(engine, &AlterTableSpec { original: after.clone(), desired: blocked }).err().map(|e| e.to_string());
    assert!(refused.is_some_and(|m| m.contains("rename column \"owner_id\"")));

    // ---- DuckDB cannot add a foreign key later: said by name, nothing run ----
    let mut with_fk = after.clone();
    with_fk.foreign_keys.push(ForeignKeySpec { name: Some("extra".into()), original_name: None, columns: vec!["owner_id".into()], ref_table: t("owners"), ref_columns: vec!["id".into()], on_delete: FkAction::NoAction, on_update: FkAction::NoAction });
    let refused = alter_table(engine, &AlterTableSpec { original: after.clone(), desired: with_fk }).err().map(|e| e.to_string());
    assert!(refused.is_some_and(|m| m.starts_with("DuckDB cannot add or drop a foreign key")));

    // ---- duplicate with rows: the copy's sequence continues past the copied ids ----
    let copy = DuplicateSpec { source: t("people"), target: t("people_copy"), include_data: true, include_indexes: false };
    let statements = duplicate_table(engine, &copy, &after).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::DuplicateTable { spec: copy }, statements).await;
    assert_eq!(scalar(&db, "SELECT count(*) FROM people_copy").await, "2");
    db.execute("INSERT INTO people_copy (email, owner_id) VALUES ('c@x', 1)", 10).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(scalar(&db, "SELECT max(id) FROM people_copy").await, "3");

    // ---- schemas: create and drop ----
    let create_schema = NamespaceSpec { kind: NamespaceKind::Schema, name: "staging".into(), op: NamespaceOp::Create { owner: None, if_not_exists: true } };
    let statements = namespace(engine, &create_schema).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::Namespace { spec: create_schema }, statements).await;
    assert_eq!(scalar(&db, "SELECT count(*) FROM information_schema.schemata WHERE schema_name = 'staging'").await, "1");
    let drop_schema = NamespaceSpec { kind: NamespaceKind::Schema, name: "staging".into(), op: NamespaceOp::Drop { cascade: true, if_exists: false } };
    let statements = namespace(engine, &drop_schema).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::Namespace { spec: drop_schema }, statements).await;
    assert_eq!(scalar(&db, "SELECT count(*) FROM information_schema.schemata WHERE schema_name = 'staging'").await, "0");

    // ---- truncate and drop ----
    let spec = RemoveSpec { table: t("people_copy"), kind: TableKind::Table, action: RemoveAction::Truncate, cascade: false, if_exists: false };
    let statements = remove_table(engine, &spec).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::RemoveTable { spec }, statements).await;
    assert_eq!(scalar(&db, "SELECT count(*) FROM people_copy").await, "0");
    let spec = RemoveSpec { table: t("people_copy"), kind: TableKind::Table, action: RemoveAction::Drop, cascade: false, if_exists: true };
    let statements = remove_table(engine, &spec).unwrap_or_else(|e| panic!("{e}"));
    run(&db, engine, &DdlOperation::RemoveTable { spec }, statements).await;
    assert!(!names(&db).await.contains(&"people_copy".to_string()));
}
