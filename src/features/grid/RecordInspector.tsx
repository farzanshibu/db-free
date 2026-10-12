// SOT: record-inspector, row-inspector, inspector-collapse, inspector-width, hex-dump, cell-text, sql-literal, inspector-copy-menu, multi-row-inspector, inspector-open-state
import { useCallback, useRef } from "react";
import type { ExportFormat, TableRef, Value } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { formatCell } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { Segmented } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Resizer } from "@/components/global/Resizer";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { COPY_FORMATS, copiedLabel, copyLabel, rowsAsText } from "./copyFormats";
import { InspectorFields } from "./InspectorFields";
import { InspectorJson } from "./InspectorJson";
import { InspectorSql } from "./InspectorSql";
import type { InspectorColumn, InspectorEdit } from "./inspectorValues";

export type { InspectorColumn, InspectorEdit } from "./inspectorValues";

/// Dragging the splitter narrower than this folds the inspector into its rail.
const INSPECTOR_FOLD_AT = 200;

// WHAT:  The inspector's folded state, shared by every grid that shows one.
// WHY:   Folding it in a table tab and finding it open in the next query result
//        reads as the app forgetting; the workbench keeps one flag (`inspectorOpen`,
//        flipped by the toolbar button, the shortcut and the layout presets).
// HOW:   Folded = narrow rail; the selection survives so it reopens on the same cell.
// WHERE: src/stores/workspace.ts (inspectorOpen, inspectorWidth)
export function useInspectorCollapsed(): [boolean, (collapsed: boolean) => void, () => void] {
  const open = useWorkspace((s) => s.inspectorOpen);
  const setOpen = useWorkspace((s) => s.setInspectorOpen);
  const toggle = useWorkspace((s) => s.toggleInspector);
  const setCollapsed = useCallback((collapsed: boolean) => setOpen(!collapsed), [setOpen]);
  return [!open, setCollapsed, toggle];
}

interface RecordInspectorProps {
  columns: readonly InspectorColumn[];
  /// The row of the clicked cell.
  row: readonly Value[];
  column: InspectorColumn;
  value: Value;
  /// The table the row came from; null for a result that is not one table's rows.
  table: TableRef | null;
  tabs: readonly string[];
  activeTab: string;
  onTab: (t: string) => void;
  collapsed: boolean;
  onToggle: () => void;
  onClose: () => void;
  /// Every row it speaks for when several are selected; omitted = just `row`.
  rows?: readonly (readonly Value[])[] | undefined;
  /// Present = the Fields and JSON tabs edit, and the SQL tab asks the connection.
  edit?: InspectorEdit | undefined;
  /// Without `edit`, the connection the SQL tab can still ask for dialect-correct statements.
  connectionId?: string | undefined;
  /// The row still holds shortened cells; their full values are on the way.
  partial?: boolean | undefined;
  /// Names the row(s) shown. Unsaved edits in the form are dropped when it changes, so a
  /// half-typed value never lands on the next row; omitted = the form never drops them.
  identity?: string | undefined;
  onCopied?: ((what: string) => void) | undefined;
  onError?: ((message: string) => void) | undefined;
}

// WHAT:  Record inspector with Fields / JSON / SQL tabs (order from settings).
//        Fields edit the row as a typed form (NULL toggle, Update / Reset / Delete),
//        JSON edits it as an object (Format), SQL writes SELECT / INSERT / UPDATE /
//        DELETE for it — and each handles several selected rows at once. "Copy row"
//        puts it on the clipboard as JSON, CSV, TSV, Markdown, NDJSON or SQL INSERT.
//        Folds into a narrow rail from the toolbar button, the header chevron, ⌘L,
//        or by dragging the splitter past the minimum width; the rail expands
//        it again. Width and folded state live in the workspace store.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
export function RecordInspector({ columns, row, column, value, table, tabs, activeTab, onTab, collapsed, onToggle, onClose, rows, edit, connectionId, partial = false, identity, onCopied, onError }: RecordInspectorProps) {
  const current = tabs.includes(activeTab) ? activeTab : (tabs[0] ?? "fields");
  const shownRows = rows !== undefined && rows.length > 0 ? rows : [row];
  const target = table === null ? `"query_result"` : `${table.schema ? `"${table.schema}".` : ""}"${table.name}"`;
  const insertSql = shownRows.map((r) => `INSERT INTO ${target} (${columns.map((c) => `"${c.name}"`).join(", ")})\nVALUES (${r.map((v) => sqlLiteral(v)).join(", ")});`).join("\n");
  const width = useWorkspace((s) => s.inspectorWidth);
  const setWidth = useWorkspace((s) => s.setInspectorWidth);
  // The width the pointer asks for, unclamped, for the length of one drag: the store keeps
  // the width inside its limits, and a drag past the minimum has to be able to fold the panel.
  const dragWidth = useRef<number | null>(null);

  const handleResize = useCallback(
    (delta: number) => {
      const next = (dragWidth.current ?? useWorkspace.getState().inspectorWidth) - delta;
      if (next < INSPECTOR_FOLD_AT) {
        dragWidth.current = null;
        onToggle();
        return;
      }
      dragWidth.current = next;
      setWidth(next);
    },
    [onToggle, setWidth],
  );

  const copyRows = async (format: ExportFormat) => {
    try {
      const target = connectionId !== undefined && table !== null ? { connectionId, table } : null;
      const text = await rowsAsText(format, columns, shownRows, target, insertSql);
      await navigator.clipboard.writeText(text);
      onCopied?.(copiedLabel(shownRows.length, format));
    } catch (raw) {
      onError?.(normalizeError(raw).message);
    }
  };

  if (collapsed) {
    return (
      <aside className="flex w-8 shrink-0 flex-col items-center gap-1 border-l border-border/40 glass-sidebar py-1 select-none">
        <IconButton icon="chevron-left" label="Expand inspector" onClick={onToggle} size={13} className="size-6 min-w-6" />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Expand inspector for ${column.name}`}
              onClick={onToggle}
              className="h-auto min-h-0 w-6 min-w-0 flex-1 overflow-hidden rounded-md px-0 py-1.5 font-mono text-[11px] whitespace-nowrap text-muted [writing-mode:vertical-rl] rotate-180 hover:text-foreground"
            >
              {column.name}
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            {column.name} · {column.dataType}
          </TooltipContent>
        </Tooltip>
        <Button variant="ghost" size="icon-sm" aria-label="Close inspector" onClick={onClose}><Icon name="x" /></Button>
      </aside>
    );
  }

  return (
    <aside className="relative flex shrink-0 flex-col border-l border-border/40 glass-sidebar select-none" style={{ width }}>
      <Resizer
        direction="horizontal"
        onResize={handleResize}
        onDragEnd={() => {
          dragWidth.current = null;
        }}
        className="absolute -left-1 top-0 bottom-0"
      />
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border/40 glass-header text-xs">
        {shownRows.length > 1 ? (
          <span className="truncate font-semibold text-foreground tracking-tight">{shownRows.length} rows</span>
        ) : (
          <>
            <span className="truncate font-semibold text-foreground tracking-tight">{column.name}</span>
            <Badge size="sm" variant="soft" className="font-mono text-[10px]">
              {column.dataType}
            </Badge>
          </>
        )}
        <span className="ml-auto flex items-center gap-0.5">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="xs" aria-label="Copy row" className="gap-1 rounded-md px-1.5 text-muted hover:text-foreground">
                <Icon name="copy" size={12} />
                Copy
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-40 glass-modal rounded-xl">
              <DropdownMenuGroup>
                {COPY_FORMATS.map((format) => (
                  <DropdownMenuItem key={format} textValue={copyLabel(format)} onSelect={() => void copyRows(format)}>
                    <span>
                      {shownRows.length === 1 ? "Row" : "Rows"} as {copyLabel(format)}
                    </span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
          <IconButton icon="chevron-right" label="Collapse inspector" onClick={onToggle} size={13} className="size-6 min-w-6" />
          <Button variant="ghost" size="icon-sm" aria-label="Close inspector" onClick={onClose}><Icon name="x" /></Button>
        </span>
      </div>
      <div className="px-2.5 py-1.5">
        <Segmented label="Inspector tab" value={current} onChange={onTab} options={tabs.map((t) => ({ value: t, label: t.toUpperCase() }))} />
      </div>
      {current === "fields" ? (
        <InspectorFields key={identity} columns={columns} rows={shownRows} focus={column.name} edit={edit} partial={partial} />
      ) : current === "json" ? (
        <InspectorJson key={identity} columns={columns} rows={shownRows} edit={edit} partial={partial} />
      ) : current === "sql" ? (
        <InspectorSql
          connectionId={edit?.connectionId ?? connectionId}
          table={table}
          columns={columns}
          keyColumns={edit?.keyColumns ?? columns.filter((c) => c.primaryKey).map((c) => c.name)}
          rows={shownRows}
          fallback={insertSql}
          onCopied={onCopied}
        />
      ) : (
        <ScrollArea className="min-h-0 flex-1">
          <pre className="selectable p-2.5 font-mono text-[11px] whitespace-pre-wrap text-foreground">{inspectorBody(value)}</pre>
        </ScrollArea>
      )}
    </aside>
  );
}

function inspectorBody(value: Value): string {
  switch (value.t) {
    case "json":
      return JSON.stringify(value.v, null, 2);
    case "bytes":
      return hexDump(value.v);
    case "null":
    case "bool":
    case "int":
    case "float":
    case "decimal":
    case "text":
    case "date_time":
    case "unsupported":
      return formatCell(value).text;
  }
}

function hexDump(base64: string): string {
  let binary = "";
  try {
    binary = atob(base64);
  } catch {
    return base64;
  }
  const lines: string[] = [];
  for (let i = 0; i < binary.length; i += 16) {
    const chunk = binary.slice(i, i + 16);
    const hex = Array.from(chunk, (ch) => ch.charCodeAt(0).toString(16).padStart(2, "0")).join(" ");
    const ascii = Array.from(chunk, (ch) => (ch.charCodeAt(0) >= 32 && ch.charCodeAt(0) < 127 ? ch : ".")).join("");
    lines.push(`${i.toString(16).padStart(8, "0")}  ${hex.padEnd(47)}  ${ascii}`);
  }
  return lines.join("\n");
}

/// A cell as the inspector and the FK lookup show it: JSON compact, NULL spelled out.
export function cellText(value: Value | undefined): string {
  if (value === undefined) return "";
  return value.t === "json" ? JSON.stringify(value.v) : value.t === "null" ? "NULL" : formatCell(value).text;
}

/// A value as a SQL literal for the INSERT previews (quotes doubled).
export function sqlLiteral(value: Value | undefined): string {
  if (value === undefined || value.t === "null") return "NULL";
  switch (value.t) {
    case "bool":
      return value.v ? "TRUE" : "FALSE";
    case "int":
    case "float":
      return String(value.v);
    case "decimal":
      return value.v;
    case "json":
      return `'${JSON.stringify(value.v).replace(/'/g, "''")}'`;
    case "text":
    case "bytes":
    case "date_time":
    case "unsupported":
      return `'${value.v.replace(/'/g, "''")}'`;
  }
}
