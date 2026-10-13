// SOT: diagrams-service, diagram-customizations-service, diagram-export-file, diagram-ddl-glue

use crate::error::{AppError, AppResult};
use crate::model::{DiagramBody, DiagramCustomization, DiagramCustomizationKey, DiagramDdl, DiagramExportFormat};
use crate::services::diagram_ddl;
use crate::state::AppState;
use base64::Engine as _;
use std::path::{Path, PathBuf};

// WHAT:  The diagram use cases that are not a document save: the memory of what
//        the user did to an ER diagram, the engine-correct SQL for a designed
//        diagram, and writing an exported image to disk.
// WHY:   Saving and versioning a diagram goes through the document services; these
//        three have a store table, a dialect, or a file to deal with and live apart.
// WHERE: src-tauri/src/commands/diagrams.rs, src/features/diagrams

/// The all-schemas diagram has no schema; the store keys it by the empty string.
fn schema_key(schema: Option<&str>) -> String {
    schema.map(str::trim).unwrap_or_default().to_string()
}

pub fn list_customizations(state: &AppState, connection_id: &str, schema: Option<&str>) -> AppResult<Vec<DiagramCustomization>> {
    let schema = schema_key(schema);
    state.with_store(|store| store.list_diagram_customizations(connection_id, &schema))
}

pub fn save_customizations(
    state: &AppState,
    connection_id: &str,
    schema: Option<&str>,
    upserts: &[DiagramCustomization],
    removals: &[DiagramCustomizationKey],
) -> AppResult<()> {
    if upserts.iter().any(|u| u.entity_key.trim().is_empty()) || removals.iter().any(|r| r.entity_key.trim().is_empty()) {
        return Err(AppError::invalid_input("A customization needs the table or relation it belongs to."));
    }
    let schema = schema_key(schema);
    state.with_store(|store| {
        store.get_connection(connection_id)?;
        store.save_diagram_customizations(connection_id, &schema, upserts, removals)
    })
}

// WHAT:  The SQL for a diagram in the dialect of its connection. A diagram without
//        a connection, or whose connection is gone, gets ANSI SQL.
pub fn ddl(state: &AppState, connection_id: Option<&str>, body: &DiagramBody, table_id: Option<&str>) -> DiagramDdl {
    let engine = connection_id.and_then(|id| state.with_store(|store| store.get_connection(id)).ok()).map(|c| c.engine);
    diagram_ddl::generate(engine, body, table_id)
}

// ------------------------------------------------------------------- export

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

// WHAT:  The bytes to write: PNG arrives base64-encoded (the webview draws the
//        image on a canvas), SVG as markup. Each is checked to be what it claims,
//        so a bad call cannot write an arbitrary file under a `.png` name.
fn export_bytes(format: DiagramExportFormat, data: &str) -> AppResult<Vec<u8>> {
    match format {
        DiagramExportFormat::Svg => {
            if !data.contains("<svg") {
                return Err(AppError::invalid_input("That is not an SVG document."));
            }
            Ok(data.as_bytes().to_vec())
        }
        DiagramExportFormat::Png => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|e| AppError::invalid_input(format!("The image could not be decoded: {e}")))?;
            if !bytes.starts_with(&PNG_SIGNATURE) {
                return Err(AppError::invalid_input("That is not a PNG image."));
            }
            Ok(bytes)
        }
    }
}

fn with_extension(path: &Path, format: DiagramExportFormat) -> PathBuf {
    let wanted = format.extension();
    match path.extension() {
        Some(ext) if ext.eq_ignore_ascii_case(wanted) => path.to_path_buf(),
        _ => path.with_extension(wanted),
    }
}

// WHAT:  Writes an exported diagram image to the path the user picked in the save dialog.
// RETURNS: The path actually written (the extension is forced, as for .sql scripts).
pub fn save_export(path: &Path, format: DiagramExportFormat, data: &str) -> AppResult<String> {
    let bytes = export_bytes(format, data)?;
    let target = with_extension(path, format);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::invalid_input(format!("{} is not a folder.", parent.display())));
        }
    }
    std::fs::write(&target, bytes).map_err(AppError::internal)?;
    Ok(target.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
    }

    fn png_base64() -> String {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(b"rest of the image");
        b64(&bytes)
    }

    #[test]
    fn the_all_schemas_diagram_is_keyed_by_the_empty_string() {
        assert_eq!(schema_key(None), "");
        assert_eq!(schema_key(Some(" public ")), "public");
    }

    #[test]
    fn exports_must_be_what_they_claim() {
        assert!(export_bytes(DiagramExportFormat::Png, &png_base64()).is_ok());
        assert!(export_bytes(DiagramExportFormat::Png, "not base64!!").is_err());
        assert!(export_bytes(DiagramExportFormat::Png, &b64(b"GIF89a")).is_err());
        assert!(export_bytes(DiagramExportFormat::Svg, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").is_ok());
        assert!(export_bytes(DiagramExportFormat::Svg, "#!/bin/sh").is_err());
    }

    #[test]
    fn the_extension_is_forced_but_a_typed_one_is_kept() {
        assert!(with_extension(Path::new("/tmp/erd"), DiagramExportFormat::Png).ends_with("erd.png"));
        assert!(with_extension(Path::new("/tmp/erd.PNG"), DiagramExportFormat::Png).ends_with("erd.PNG"));
        assert!(with_extension(Path::new("/tmp/erd.png"), DiagramExportFormat::Svg).ends_with("erd.svg"));
    }

    #[test]
    fn writes_the_file_and_refuses_a_missing_folder() {
        let dir = std::env::temp_dir().join(format!("db-free-diagram-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let written = save_export(&dir.join("shop"), DiagramExportFormat::Svg, "<svg/>").unwrap_or_else(|e| panic!("{e}"));
        assert!(written.ends_with("shop.svg"), "{written}");
        assert_eq!(std::fs::read_to_string(&written).unwrap_or_default(), "<svg/>");
        assert!(save_export(&dir.join("nope").join("shop"), DiagramExportFormat::Svg, "<svg/>").is_err());
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    }
}
