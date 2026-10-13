// SOT: open-sql-file-flow, sql-file-to-query-tab
import { ipc, normalizeError } from "@/lib/ipc";
import { pickSqlOpenPath } from "@/lib/native";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  Asks the user for a .sql file and opens it in a new query tab on `connectionId`.
// WHY:   Scripts live in files. The editor's Open button, ⌘O and the command
//        palette all do this one thing, so it is written once.
// HOW:   The OS dialog picks the path; Rust reads the file (the webview may not)
//        and flags text it had to repair. Cancelling the dialog does nothing.
// WHERE: src-tauri/src/services/scripts.rs (read), src/lib/native.ts (pickSqlOpenPath)
export async function openSqlFileInNewTab(connectionId: string): Promise<void> {
  const workspace = useWorkspace.getState();
  try {
    const path = await pickSqlOpenPath();
    if (path === null) return;
    const file = await ipc("read_sql_file", { path });
    workspace.openQuery(connectionId, file.sql, file.name);
    if (file.lossy) workspace.showInfo(`Some characters in ${file.name} could not be decoded and were replaced.`);
  } catch (raw) {
    workspace.showError(normalizeError(raw));
  }
}
