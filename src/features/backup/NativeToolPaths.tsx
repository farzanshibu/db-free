// SOT: native-tool-path-settings, native-tool-overrides-ui
import { useEffect, useState } from "react";
import type { AppSettings, NativeTool, NativeToolStatus } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { pickBackupSource } from "@/lib/native";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Field } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Spinner } from "@/components/ui/spinner";

type ToolPaths = AppSettings["nativeToolPaths"];

// WHAT:  Settings → Advanced: where pg_dump, mysqldump, mongodump… live, for
//        when they are not on PATH.
// WHY:   Backup / Restore drives the engines' own client tools. A GUI app on
//        macOS does not see the shell's PATH, and Windows installers often skip
//        it, so "not found" needs a fix that is not "edit your environment".
// HOW:   Edits the settings draft like every other row; the saved value is
//        what `detect_native_tools` resolved against, shown beside each field.
// WHERE: src-tauri/src/integrations/native_tools.rs (locate), src/features/backup/BackupDialog.tsx
export function NativeToolPaths({ value, onChange }: { value: ToolPaths; onChange: (next: ToolPaths) => void }) {
  const [tools, setTools] = useState<NativeToolStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        setTools((await ipc("detect_native_tools", { connectionId: null })).tools);
      } catch (raw) {
        setError(errorMessage(normalizeError(raw)));
      }
    })();
  }, []);

  const set = (tool: NativeTool, path: string) => {
    const next: ToolPaths = {};
    for (const t of tools ?? []) {
      const v = t.tool === tool ? path : value[t.tool];
      if (v !== undefined && v.trim().length > 0) next[t.tool] = v;
    }
    onChange(next);
  };

  if (error !== null) return <p className="text-xs text-danger">{error}</p>;
  if (tools === null) {
    return (
      <div className="flex items-center gap-2 text-xs text-muted">
        <Spinner size="sm" /> Looking for backup tools…
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2.5">
      {tools.map((t) => (
        <div key={t.tool} className="flex items-end gap-2">
          <Field
            label={t.tool}
            value={value[t.tool] ?? ""}
            onChange={(path) => set(t.tool, path)}
            placeholder={t.path !== null && !t.overridden ? `Found: ${t.path}` : `Not found on PATH — ${t.package}`}
            mono
            description={t.overridden ? (t.path !== null ? `Using ${t.path}${t.version !== null ? ` (${t.version})` : ""}` : "The saved path has no such program.") : undefined}
          />
          <Icon name={t.path !== null ? "check" : "alert"} size={14} className={cn("mb-2.5 shrink-0", t.path !== null ? "text-success" : "text-warning")} />
          <IconButton
            icon="folder"
            label={`Choose the folder holding ${t.tool}`}
            onClick={() => {
              void (async () => {
                const picked = await pickBackupSource(true);
                if (picked !== null) set(t.tool, picked);
              })();
            }}
          />
        </div>
      ))}
    </div>
  );
}
