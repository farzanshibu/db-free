// SOT: native-dialogs, file-picker, directory-picker, sql-file-save-picker, object-save-picker, backup-save-picker, backup-open-picker, ssh-key-picker, diagram-image-save-picker
import { open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
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

// WHAT:  Picks an SSH private key file (id_ed25519, id_rsa, *.pem, *.key …).
// WHY:   No extension filter: OpenSSH keys usually have none. Only the path is
//        kept; Rust reads the key when the tunnel opens.
export async function pickPrivateKeyFile(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, title: "Choose SSH private key" });
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

// WHAT:  Asks the OS which .sql file to open into a new query tab.
// WHY:   The webview may not read files itself; it only learns the path the
//        user chose, and Rust reads the file.
// WHERE: src-tauri/src/services/scripts.rs (read), src/features/editor/QueryPane.tsx
export async function pickSqlOpenPath(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "SQL script", extensions: ["sql", "txt"] }],
  });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Save dialog for object-storage downloads (S3 / MinIO / R2).
// WHY:   The backend writes bytes straight to disk, so the UI only picks the destination.
export async function pickSaveFile(suggestedName: string): Promise<string | null> {
  const picked = await save({ defaultPath: suggestedName });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Where to write a backup. `extension` is empty for a pg_dump directory
//        backup, which is a folder pg_dump creates rather than a file.
// WHY:   The save dialog asks before replacing an existing file, which is the
//        only consent an overwrite of a previous backup gets.
// WHERE: src/features/backup/BackupDialog.tsx
export async function pickBackupSavePath(defaultName: string, extension: string, label: string): Promise<string | null> {
  const picked = await save({
    defaultPath: extension.length > 0 ? `${defaultName}.${extension}` : defaultName,
    filters: extension.length > 0 ? [{ name: label, extensions: [extension] }] : [],
  });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  The backup to restore from: a file, or a pg_dump directory backup.
export async function pickBackupSource(directory: boolean): Promise<string | null> {
  const picked = await open({ multiple: false, directory });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Where to write an exported diagram image (PNG or SVG).
// WHY:   The webview cannot write files; it draws the image, Rust writes it, and
//        the save dialog (which asks before replacing a file) is the only thing
//        that chooses the path.
// WHERE: src/features/diagrams/exportDiagram.ts, src-tauri/src/services/diagrams.rs
export async function pickDiagramSavePath(defaultName: string, format: "png" | "svg"): Promise<string | null> {
  const picked = await save({
    defaultPath: defaultName.toLowerCase().endsWith(`.${format}`) ? defaultName : `${defaultName}.${format}`,
    filters: [{ name: format === "png" ? "PNG image" : "SVG image", extensions: [format] }],
  });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  The file to import: a data file (CSV, TSV, text, JSON) or a SQL script.
// WHY:   The import tab sorts out what a data file holds by reading it, so its
//        picker offers every kind of data file at once instead of asking the
//        user to announce the format first.
// WHERE: src/features/transfer/ImportPanel.tsx
export async function pickTransferFile(kind: "data" | "sql"): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory: false,
    filters:
      kind === "sql"
        ? [{ name: "SQL script", extensions: ["sql", "txt"] }]
        : [{ name: "Data file", extensions: ["csv", "tsv", "txt", "json"] }],
  });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Calls `handler` with the paths of files dropped on the window.
// WHY:   Dropping a file is the quickest way into the import tab; the webview
//        only reports the paths through Tauri's drag-and-drop event.
// HOW:   Returns the function that stops listening.
export async function onFilesDropped(handler: (paths: readonly string[]) => void): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop") handler(event.payload.paths);
  });
}

// WHAT:  Pickers for a custom theme file (Settings → Themes → Import / Export).
// WHY:   Themes travel as small .json files; the OS dialog is the only consent the
//        read or the write gets, and Rust does both (services/theme_files.rs).
export async function pickThemeFile(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: [{ name: "Theme file", extensions: ["json"] }] });
  return typeof picked === "string" ? picked : null;
}

export async function pickThemeSavePath(themeName: string): Promise<string | null> {
  const picked = await save({
    defaultPath: themeName.toLowerCase().endsWith(".json") ? themeName : `${themeName}.json`,
    filters: [{ name: "Theme file", extensions: ["json"] }],
  });
  return typeof picked === "string" ? picked : null;
}
