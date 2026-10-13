// SOT: theme-file-service, theme-file-read, theme-file-write

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// Largest theme file read or written. A theme is a few kilobytes of colours;
/// anything bigger is not a theme.
pub const MAX_THEME_BYTES: u64 = 64 * 1024;

// WHAT:  Writes a theme's JSON to a file the user picked.
// WHY:   The webview may not write files itself; the UI hands the text over and
//        Rust writes it, at the path the OS save dialog returned.
// HOW:   The extension is forced to `.json` so a name typed without one still
//        lands as a theme file, and the size is capped like the reader's.
// WHERE: src/features/settings/ThemesSection.tsx, src/lib/native.ts (pickers)
pub fn write(path: &Path, json: &str) -> AppResult<String> {
    if u64::try_from(json.len()).unwrap_or(u64::MAX) > MAX_THEME_BYTES {
        return Err(AppError::invalid_input("A theme file cannot be larger than 64 KB."));
    }
    let target = with_json_extension(path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::invalid_input(format!("{} is not a folder.", parent.display())));
        }
    }
    std::fs::write(&target, json).map_err(AppError::internal)?;
    Ok(target.to_string_lossy().into_owned())
}

// WHAT:  Reads a theme file the user picked, as text.
// HOW:   Refuses anything over the cap before reading it, so a stray huge file
//        picked by mistake never lands in memory.
pub fn read(path: &Path) -> AppResult<String> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::invalid_input(format!("Cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(AppError::invalid_input(format!("{} is not a file.", path.display())));
    }
    if meta.len() > MAX_THEME_BYTES {
        return Err(AppError::invalid_input("That file is larger than 64 KB, so it is not a theme."));
    }
    std::fs::read_to_string(path).map_err(|e| AppError::invalid_input(format!("Cannot read {}: {e}", path.display())))
}

fn with_json_extension(path: &Path) -> PathBuf {
    match path.extension() {
        Some(ext) if ext.eq_ignore_ascii_case("json") => path.to_path_buf(),
        _ => path.with_extension("json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-themes-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[test]
    fn writes_and_reads_back_with_a_json_extension() {
        let dir = scratch("roundtrip");
        let written = write(&dir.join("mine"), "{\"a\":1}").unwrap_or_else(|e| panic!("{e}"));
        assert!(written.ends_with("mine.json"), "{written}");
        assert_eq!(read(Path::new(&written)).unwrap_or_default(), "{\"a\":1}");
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn oversized_files_are_refused_both_ways() {
        let dir = scratch("oversized");
        let big = "x".repeat(usize::try_from(MAX_THEME_BYTES).unwrap_or(0) + 1);
        assert!(write(&dir.join("big.json"), &big).is_err());
        std::fs::write(dir.join("big.json"), &big).unwrap_or_else(|e| panic!("{e}"));
        assert!(read(&dir.join("big.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn a_missing_folder_is_reported() {
        assert!(write(Path::new("/definitely/not/here/theme.json"), "{}").is_err());
    }
}
