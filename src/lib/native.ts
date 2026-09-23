// SOT: native-dialogs, file-picker, directory-picker
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

// WHAT:  Save dialog for object-storage downloads (S3 / MinIO / R2).
// WHY:   The backend writes bytes straight to disk, so the UI only picks the destination.
export async function pickSaveFile(suggestedName: string): Promise<string | null> {
  const picked = await save({ defaultPath: suggestedName });
  return typeof picked === "string" ? picked : null;
}
