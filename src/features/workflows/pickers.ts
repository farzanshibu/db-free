// SOT: workflow-file-pickers, read-file-picker, save-file-picker
import { open, save } from "@tauri-apps/plugin-dialog";
import type { FileFormat, SaveFormat } from "@/lib/bindings";

// WHAT:  The native file dialogs the Read / Save file nodes use.
// WHY:   The webview may not touch the disk. The user picks a path here and the core
//        reads or writes it when the workflow runs, so the dialog is the only consent
//        a file path gets.
// WHERE: src/features/workflows/forms/{DataForms,OutputForms}.tsx, src/features/workflows/WorkflowTab.tsx

const READ_EXTENSIONS = {
  csv: ["csv", "txt"],
  tsv: ["tsv", "tab", "txt"],
  json: ["json"],
  ndjson: ["ndjson", "jsonl", "json"],
} satisfies Record<FileFormat, string[]>;

export const SAVE_EXTENSIONS = {
  csv: "csv",
  tsv: "tsv",
  json: "json",
  ndjson: "ndjson",
  markdown: "md",
  sql: "sql",
} satisfies Record<SaveFormat, string>;

export async function pickDataFile(format: FileFormat): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: [{ name: format.toUpperCase(), extensions: READ_EXTENSIONS[format] }] });
  return typeof picked === "string" ? picked : null;
}

// WHAT:  Where to write the output. The save dialog asks before replacing a file.
export async function pickOutputFile(suggestedName: string, format: SaveFormat): Promise<string | null> {
  const extension = SAVE_EXTENSIONS[format];
  const base = suggestedName.trim() === "" ? "output" : suggestedName.trim();
  const picked = await save({
    defaultPath: base.toLowerCase().endsWith(`.${extension}`) ? base : `${base}.${extension}`,
    filters: [{ name: format.toUpperCase(), extensions: [extension] }],
  });
  return typeof picked === "string" ? picked : null;
}
