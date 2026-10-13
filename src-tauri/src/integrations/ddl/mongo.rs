// SOT: ddl-mongo-commands, mongo-create-collection, mongo-drop-collection, mongo-rename-collection, mongo-drop-database

use super::dialect::check_name;
use crate::error::{AppError, AppResult};
use crate::model::{DuplicateSpec, NamespaceOp, NamespaceSpec, RemoveAction, RemoveSpec, TableKind, TableRef, TableSpec};

// WHAT:  The MongoDB side of the schema forms: a "table" is a collection, a
//        "schema" is the database it lives in, and every statement is one command
//        document in the console's JSON form (`{"create": "orders"}`).
// WHY:   MongoDB has no SQL and no columns; creating, emptying, dropping and
//        renaming a collection are plain commands, so they fit the same preview →
//        run → refresh flow as the SQL engines' forms.
// HOW:   Documents are written by hand rather than through serde_json so their key
//        order is fixed (the preview and the tests read the same text). A
//        collection in another database carries the `"$db"` routing key the
//        adapter strips before it reaches the driver.
// WHERE: src-tauri/src/integrations/mongodb.rs (`parse_command` reads these)

fn json_text(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// `{"key": value, …, "$db": "db"}`; `fields` hold ready-made JSON values.
fn command(fields: &[(&str, String)], db: Option<&str>) -> String {
    let mut parts: Vec<String> = fields.iter().map(|(k, v)| format!("{}:{v}", json_text(k))).collect();
    if let Some(db) = db.filter(|d| !d.is_empty()) {
        parts.push(format!("\"$db\":{}", json_text(db)));
    }
    format!("{{{}}}", parts.join(","))
}

fn database_of(table: &TableRef) -> Option<&str> {
    table.schema.as_deref().filter(|s| !s.is_empty())
}

pub fn check_collection(name: &str) -> AppResult<()> {
    check_name("collection", name)?;
    if name.contains('$') || name.contains('\0') {
        return Err(AppError::invalid_input(format!("The collection name \"{name}\" may not contain \"$\".")));
    }
    if name.starts_with("system.") {
        return Err(AppError::invalid_input("Collection names starting with \"system.\" are reserved by MongoDB."));
    }
    Ok(())
}

pub fn create_collection(spec: &TableSpec) -> AppResult<Vec<String>> {
    check_collection(&spec.table.name)?;
    Ok(vec![command(&[("create", json_text(&spec.table.name))], database_of(&spec.table))])
}

pub fn remove(spec: &RemoveSpec) -> AppResult<Vec<String>> {
    check_name("collection", &spec.table.name)?;
    let db = database_of(&spec.table);
    Ok(vec![match spec.action {
        RemoveAction::Drop => command(&[("drop", json_text(&spec.table.name))], db),
        RemoveAction::Truncate => {
            if spec.kind == TableKind::View {
                return Err(AppError::invalid_input("A view holds no documents of its own to delete."));
            }
            command(&[("delete", json_text(&spec.table.name)), ("deletes", "[{\"q\":{},\"limit\":0}]".to_string())], db)
        }
    }])
}

/// Renaming a collection is an `admin` command over full `db.collection` names.
pub fn rename_collection(from: &TableRef, to: &str) -> AppResult<Vec<String>> {
    check_collection(to)?;
    let Some(db) = database_of(from) else {
        return Err(AppError::invalid_input("MongoDB needs the collection's database name to rename it."));
    };
    Ok(vec![command(
        &[("renameCollection", json_text(&format!("{db}.{}", from.name))), ("to", json_text(&format!("{db}.{to}")))],
        Some("admin"),
    )])
}

/// A copy of a collection: its documents (`$out`) or just an empty collection.
pub fn duplicate(spec: &DuplicateSpec) -> AppResult<Vec<String>> {
    check_collection(&spec.target.name)?;
    if spec.source == spec.target {
        return Err(AppError::invalid_input("Pick a different name for the copy."));
    }
    let db = database_of(&spec.source);
    if spec.include_data {
        let out = if database_of(&spec.target) == db || database_of(&spec.target).is_none() {
            json_text(&spec.target.name)
        } else {
            format!("{{\"db\":{},\"coll\":{}}}", json_text(database_of(&spec.target).unwrap_or_default()), json_text(&spec.target.name))
        };
        return Ok(vec![command(
            &[("aggregate", json_text(&spec.source.name)), ("pipeline", format!("[{{\"$out\":{out}}}]")), ("cursor", "{}".to_string())],
            db,
        )]);
    }
    Ok(vec![command(&[("create", json_text(&spec.target.name))], database_of(&spec.target).or(db))])
}

pub fn namespace(spec: &NamespaceSpec) -> AppResult<Vec<String>> {
    check_name("database", &spec.name)?;
    match &spec.op {
        NamespaceOp::Drop { .. } => Ok(vec![command(&[("dropDatabase", "1".to_string())], Some(&spec.name))]),
        NamespaceOp::Create { .. } => Err(AppError::invalid_input("MongoDB creates a database when its first collection is created.")),
        NamespaceOp::Rename { .. } => Err(AppError::invalid_input("MongoDB cannot rename a database; copy its collections into a new one and drop the old.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NamespaceKind;

    fn coll(db: Option<&str>, name: &str) -> TableRef {
        TableRef { schema: db.map(String::from), name: name.into() }
    }

    #[test]
    fn collections_are_created_dropped_emptied_and_renamed_with_commands() {
        let spec = TableSpec {
            table: coll(Some("shop"), "orders"),
            columns: Vec::new(),
            primary_key_name: None,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
            engine: None,
            order_by: Vec::new(),
            if_not_exists: false,
        };
        assert_eq!(create_collection(&spec).unwrap_or_default(), vec![r#"{"create":"orders","$db":"shop"}"#.to_string()]);
        let remove_spec = |action| RemoveSpec { table: coll(Some("shop"), "orders"), kind: TableKind::Table, action, cascade: false, if_exists: false };
        assert_eq!(remove(&remove_spec(RemoveAction::Drop)).unwrap_or_default(), vec![r#"{"drop":"orders","$db":"shop"}"#.to_string()]);
        assert_eq!(
            remove(&remove_spec(RemoveAction::Truncate)).unwrap_or_default(),
            vec![r#"{"delete":"orders","deletes":[{"q":{},"limit":0}],"$db":"shop"}"#.to_string()]
        );
        assert_eq!(
            rename_collection(&coll(Some("shop"), "orders"), "orders_v2").unwrap_or_default(),
            vec![r#"{"renameCollection":"shop.orders","to":"shop.orders_v2","$db":"admin"}"#.to_string()]
        );
        assert!(rename_collection(&coll(None, "orders"), "x").is_err());
        assert!(check_collection("a$b").is_err() && check_collection("system.users").is_err() && check_collection("ok_name").is_ok());
        let view = RemoveSpec { kind: TableKind::View, ..remove_spec(RemoveAction::Truncate) };
        assert!(remove(&view).is_err());
    }

    #[test]
    fn duplicates_copy_documents_with_out_or_make_an_empty_collection() {
        let spec = DuplicateSpec { source: coll(Some("shop"), "orders"), target: coll(Some("shop"), "orders_copy"), include_data: true, include_indexes: false };
        assert_eq!(
            duplicate(&spec).unwrap_or_default(),
            vec![r#"{"aggregate":"orders","pipeline":[{"$out":"orders_copy"}],"cursor":{},"$db":"shop"}"#.to_string()]
        );
        let empty = DuplicateSpec { include_data: false, ..spec.clone() };
        assert_eq!(duplicate(&empty).unwrap_or_default(), vec![r#"{"create":"orders_copy","$db":"shop"}"#.to_string()]);
        let elsewhere = DuplicateSpec { target: coll(Some("archive"), "orders"), ..spec.clone() };
        assert!(duplicate(&elsewhere).unwrap_or_default().first().is_some_and(|c| c.contains(r#"{"$out":{"db":"archive","coll":"orders"}}"#)));
        assert!(duplicate(&DuplicateSpec { target: spec.source.clone(), ..spec }).is_err());
    }

    #[test]
    fn only_dropping_a_database_is_possible() {
        let ns = |op| NamespaceSpec { kind: NamespaceKind::Database, name: "shop".into(), op };
        assert_eq!(namespace(&ns(NamespaceOp::Drop { cascade: false, if_exists: false })).unwrap_or_default(), vec![r#"{"dropDatabase":1,"$db":"shop"}"#.to_string()]);
        assert!(namespace(&ns(NamespaceOp::Create { owner: None, if_not_exists: false })).is_err());
        assert!(namespace(&ns(NamespaceOp::Rename { to: "x".into() })).is_err());
    }
}
