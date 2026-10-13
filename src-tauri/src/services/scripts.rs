// SOT: scripts-service, sql-file-save, editor-script-export

use crate::error::{AppError, AppResult};
use crate::model::OpenedSqlFile;
use std::path::{Path, PathBuf};

// WHAT:  Writes the editor's script to a file the user picked.
// WHY:   PRD §4.3 — a query is worth keeping outside the app: handed to a
//        colleague, committed to a repo, opened by any other tool.
// HOW:   The extension is forced to `.sql` so a name typed without one still
//        lands as a script rather than an extensionless file. The path itself
//        comes from the OS save dialog, so the user has already chosen it.
// WHERE: src/lib/native.ts (pickSqlSavePath), src-tauri/src/commands/query.rs
pub fn save(path: &Path, sql: &str) -> AppResult<String> {
    let target = with_sql_extension(path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::invalid_input(format!("{} is not a folder.", parent.display())));
        }
    }
    std::fs::write(&target, sql).map_err(AppError::internal)?;
    Ok(target.to_string_lossy().into_owned())
}

/// The largest script the editor opens. Past this a tab is the wrong tool: a dump
/// belongs in Import, which streams it statement by statement.
const MAX_SCRIPT_BYTES: u64 = 20 * 1024 * 1024;

// WHAT:  Reads a `.sql` file the user picked into the editor.
// WHY:   Opening a script from disk is the other half of saving one.
// HOW:   Refuses folders and files past `MAX_SCRIPT_BYTES`. The bytes are decoded
//        by their byte-order mark (UTF-8, or the UTF-16 SQL Server tools write),
//        else as UTF-8; text that is not valid is kept with replacement
//        characters and flagged, rather than refused.
// WHERE: src/lib/native.ts (pickSqlOpenPath), src-tauri/src/commands/query.rs
pub fn read(path: &Path) -> AppResult<OpenedSqlFile> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::invalid_input(format!("Cannot open {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(AppError::invalid_input(format!("{} is not a file.", path.display())));
    }
    if meta.len() > MAX_SCRIPT_BYTES {
        return Err(AppError::invalid_input(format!(
            "{} is {} MB; the editor opens scripts up to {} MB. Use Import to run a large dump.",
            path.display(),
            meta.len() / (1024 * 1024),
            MAX_SCRIPT_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| AppError::invalid_input(format!("Cannot read {}: {e}", path.display())))?;
    let (sql, lossy) = decode(bytes);
    let name = path.file_stem().map_or_else(|| "Query".to_string(), |stem| stem.to_string_lossy().into_owned());
    Ok(OpenedSqlFile { name, sql, lossy })
}

fn decode(bytes: Vec<u8>) -> (String, bool) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return text_of(rest.to_vec());
    }
    let utf16 = |rest: &[u8], big: bool| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|pair| if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) }).collect();
        (String::from_utf16_lossy(&units), rest.len() % 2 != 0 || String::from_utf16(&units).is_err())
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true);
    }
    text_of(bytes)
}

fn text_of(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(text) => (text, false),
        Err(err) => (String::from_utf8_lossy(err.as_bytes()).into_owned(), true),
    }
}

fn with_sql_extension(path: &Path) -> PathBuf {
    match path.extension() {
        Some(ext) if ext.eq_ignore_ascii_case("sql") => path.to_path_buf(),
        _ => path.with_extension("sql"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_script_and_forces_the_extension() {
        let dir = std::env::temp_dir().join(format!("db-free-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let written = save(&dir.join("report"), "select 1;").unwrap_or_else(|e| panic!("{e}"));
        assert!(written.ends_with("report.sql"), "{written}");
        assert_eq!(std::fs::read_to_string(&written).unwrap_or_default(), "select 1;");
        let again = save(&dir.join("report.SQL"), "select 2;").unwrap_or_else(|e| panic!("{e}"));
        assert!(again.ends_with("report.SQL"), "an extension the user typed is kept: {again}");
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn opens_a_script_whatever_its_encoding() {
        let dir = std::env::temp_dir().join(format!("db-free-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
            path
        };
        let plain = read(&write("plain.sql", "select 'é';".as_bytes())).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((plain.name.as_str(), plain.sql.as_str(), plain.lossy), ("plain", "select 'é';", false));
        let bom = read(&write("bom.sql", b"\xEF\xBB\xBFselect 1")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bom.sql, "select 1", "a UTF-8 byte-order mark is not part of the script");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("select 2".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let wide = read(&write("wide.sql", &utf16)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((wide.sql.as_str(), wide.lossy), ("select 2", false));
        let latin1 = read(&write("latin1.sql", b"select '\xE9'")).unwrap_or_else(|e| panic!("{e}"));
        assert!(latin1.lossy && latin1.sql.contains('\u{FFFD}'));
        assert!(read(&dir).is_err(), "a folder is not a script");
        assert!(read(&dir.join("missing.sql")).is_err());
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    }
}
