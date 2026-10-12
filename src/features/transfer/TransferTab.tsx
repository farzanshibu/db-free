// SOT: transfer-tab, export-ui, import-ui
import { useState } from "react";
import { Segmented } from "@/components/global/Field";
import { ExportPanel } from "./ExportPanel";
import { ImportPanel } from "./ImportPanel";

// WHAT:  Export / Import tab. Export writes ticked tables to one file (ZIP,
//        combined JSON, combined SQL) or a folder, with the options of each
//        format and a preview of the real output. Import takes a CSV, TSV or JSON
//        file into a table with column mapping, a multi-table JSON file as a
//        checklist, or a .sql script statement by statement.
// WHERE: src-tauri/src/services/{export,transfer,sql_import}.rs
export function TransferTab({ connectionId }: { connectionId: string }) {
  const [mode, setMode] = useState<"export" | "import">("export");
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border bg-surface">
        <Segmented
          label="Transfer mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "export", label: "Export" },
            { value: "import", label: "Import" },
          ]}
        />
      </div>
      {mode === "export" ? <ExportPanel connectionId={connectionId} /> : <ImportPanel connectionId={connectionId} />}
    </div>
  );
}
