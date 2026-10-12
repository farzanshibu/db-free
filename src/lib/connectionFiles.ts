// SOT: connection-file-pickers, connection-export-save-dialog, connection-import-open-dialog, file-drop-listener, connection-opened-event
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { ConnectionOpenedEvent } from "./bindings";

// WHAT:  Native pickers and OS events for connection files: where to write an
//        export, which file to import, a certificate to trust, and databases
//        the OS or a drag-drop hands the window.
// WHY:   The webview never reads or writes these files itself. It only learns a
//        path from the user (a dialog or a drop) and hands it to Rust, so a
//        decrypted password in an export never exists in the page. Wrapped here
//        so components do not import Tauri plugins directly.
// WHERE: src/features/connections/ConnectionTransferDialogs.tsx, ConnectionEvents.tsx

function isoDate(): string {
  return new Date().toISOString().slice(0, 10);
}

// WHAT:  `db-free-connections-YYYY-MM-DD.json`: the default name of an export.
export function exportFileName(): string {
  return `db-free-connections-${isoDate()}.json`;
}

export async function pickConnectionExportPath(): Promise<string | null> {
  const picked = await save({
    title: "Export connections",
    defaultPath: exportFileName(),
    filters: [{ name: "DB Free connections", extensions: ["json"] }],
  });
  return typeof picked === "string" ? picked : null;
}

export async function pickConnectionImportFile(): Promise<string | null> {
  const picked = await open({
    title: "Import connections",
    multiple: false,
    directory: false,
    filters: [
      { name: "Connections (DB Free, DB Pro, Navicat)", extensions: ["json", "dbpro", "ncx"] },
      { name: "All files", extensions: ["*"] },
    ],
  });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  A certificate or key file for a TLS setting. No extension filter: PEM
//        files are `.pem`, `.crt`, `.cer`, `.key` or nothing at all.
export async function pickCertificateFile(title: string): Promise<string | null> {
  const picked = await open({ title, multiple: false, directory: false });
  return typeof picked === "string" ? picked : null;
}

const DATABASE_EXTENSIONS: readonly string[] = ["db", "sqlite", "sqlite3", "db3"];

// WHAT:  Whether a dropped path is a SQLite database the app opens directly.
export function isDatabasePath(path: string): boolean {
  const dot = path.lastIndexOf(".");
  return dot >= 0 && DATABASE_EXTENSIONS.includes(path.slice(dot + 1).toLowerCase());
}

// WHAT:  Files dropped on the window, and whether something is being dragged over it.
// HOW:   Tauri reports drops with absolute paths (the browser's own `drop` event
//        cannot see them). `onHover` receives the dragged paths on `enter` and
//        null on `leave` / `drop`, so a drop zone can light up only for files it takes.
export function onFilesDropped(onDrop: (paths: string[]) => void, onHover?: (paths: string[] | null) => void): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === "enter") onHover?.(payload.paths);
    else if (payload.type === "leave") onHover?.(null);
    else if (payload.type === "drop") {
      onHover?.(null);
      onDrop(payload.paths);
    }
  });
}

type DropClaim = (paths: string[]) => void;
let activeClaim: DropClaim | null = null;

// WHAT:  A screen that wants dropped files for itself (the SQLite form's drop zone).
// WHY:   By default a dropped database opens as a transient connection. While the
//        form is open the same drop should fill its file field instead, so the
//        form claims drops for as long as it is mounted. Returns the release function.
export function claimFileDrops(handler: DropClaim): () => void {
  activeClaim = handler;
  return () => {
    if (activeClaim === handler) activeClaim = null;
  };
}

// WHAT:  Hands a drop to whoever claimed it; false when nobody did.
export function dispatchClaimedDrop(paths: string[]): boolean {
  if (activeClaim === null) return false;
  activeClaim(paths);
  return true;
}

// WHAT:  The OS handed the app a database file (double-click, "Open with", a second launch).
// WHERE: src-tauri/src/open_files.rs (emits "connection:opened")
export function onConnectionOpened(handler: (event: ConnectionOpenedEvent) => void): Promise<UnlistenFn> {
  return listen<ConnectionOpenedEvent>("connection:opened", (event) => { handler(event.payload); });
}
