// SOT: native-dialogs, file-picker, directory-picker, sql-file-save-picker
import { open, save } from "@tauri-apps/plugin-dialog";
import type { TransferFormat } from "./bindings";

// WHAT:  Native pickers. Wrapped so components never import a Tauri plugin directly.
export async function pickSqliteFile(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Database file", extensions: ["db", "sqlite", "sqlite3", "db3", "duckdb", "ddb", "gpkg"] }],
  });
  return typeof picked === "string" ? picked : null;
}

export async function pickDirectory(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: true });
  return typeof picked === "string" ? picked : null;
}

export async function pickImportFile(format: TransferFormat): Promise<string | null> {
  const extensions = format === "csv" ? ["csv", "txt"] : format === "json" ? ["json"] : ["sql"];
  const picked = await open({ multiple: false, directory: false, filters: [{ name: format.toUpperCase(), extensions }] });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Asks the OS where to write the editor's script.
// WHY:   The webview may not write files itself; Rust does the writing, and this
//        is the only thing that may choose the path — the user.
// WHERE: src-tauri/src/services/scripts.rs (the write), src/features/editor/QueryPane.tsx
export async function pickSqlSavePath(defaultName: string): Promise<string | null> {
  const picked = await save({
    defaultPath: defaultName.toLowerCase().endsWith(".sql") ? defaultName : `${defaultName}.sql`,
    filters: [{ name: "SQL script", extensions: ["sql"] }],
  });
  return typeof picked === "string" ? picked : null;
}
