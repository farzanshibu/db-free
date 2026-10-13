// SOT: transfer-size-format, file-size-label, elapsed-label

// WHAT:  How the export and import tabs word a size and a duration.
// WHERE: src/features/transfer/*.tsx
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function formatElapsed(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/// The last path segment, whichever separator the OS uses.
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export type FileKind = "csv" | "json" | "sql";

// WHAT:  What a dropped or chosen file is, from its extension. `.txt` is read as
//        delimited text; the import tab sniffs nothing else.
export function fileKindOf(path: string): FileKind | null {
  const extension = baseName(path).split(".").pop()?.toLowerCase();
  switch (extension) {
    case "csv":
    case "tsv":
    case "txt":
      return "csv";
    case "json":
      return "json";
    case "sql":
      return "sql";
    default:
      return null;
  }
}

export function isTsv(path: string): boolean {
  return baseName(path).toLowerCase().endsWith(".tsv");
}
