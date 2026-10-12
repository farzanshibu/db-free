// SOT: database-file-drop-zone, sqlite-form-drop-target
import { useEffect, useState } from "react";
import { claimFileDrops, onFilesDropped } from "@/lib/connectionFiles";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

interface DatabaseDropZoneProps {
  /// Called with the first dropped path.
  onPath: (path: string) => void;
  /// What the file is called for this engine ("SQLite database file", "RocksDB directory").
  noun: string;
}

// WHAT:  A drop target on the file-database form: drop `.db` / `.sqlite` /
//        `.sqlite3` (or a directory, for RocksDB) to fill the path.
// WHY:   Anywhere else in the app a dropped database opens as a transient
//        connection. On this form the user is *describing* a connection, so the
//        drop must fill the field instead: the zone claims drops for as long as
//        it is mounted, and the global handler stands aside.
// HOW:   The browser's own drop event cannot see file paths in a webview, so
//        this listens to Tauri's drag-drop events for both the highlight and
//        the claim.
// WHERE: src/lib/connectionFiles.ts (claimFileDrops), src/features/connections/ConnectionEvents.tsx
export function DatabaseDropZone({ onPath, noun }: DatabaseDropZoneProps) {
  const [over, setOver] = useState(false);

  useEffect(() => {
    const release = claimFileDrops((paths) => {
      const first = paths[0];
      if (first !== undefined) onPath(first);
    });
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void onFilesDropped(() => undefined, (paths) => { setOver(paths !== null); }).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });
    return () => {
      cancelled = true;
      release();
      unlisten?.();
    };
  }, [onPath]);

  return (
    <div
      className={cn(
        "flex items-center justify-center gap-2 rounded-lg border border-dashed px-3 py-1.5 text-xs transition-colors",
        over ? "border-accent bg-accent/10 text-foreground" : "border-border/60 text-muted",
      )}
    >
      <Icon name="database" size={14} className={over ? "text-accent" : ""} />
      {over ? `Drop to use this ${noun}` : `Drop a ${noun} here, or use Browse`}
    </div>
  );
}
