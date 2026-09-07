// SOT: scripts-service, sql-file-save, editor-script-export

use crate::error::{AppError, AppResult};
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
}
