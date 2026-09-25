// SOT: record-inspector, row-inspector, inspector-collapse, inspector-width, hex-dump, cell-text, sql-literal
import { useCallback, useEffect, useRef, useState } from "react";
import type { ColumnInfo, TableRef, Value } from "@/lib/bindings";
import { plainValue } from "@/lib/export";
import { formatCell } from "@/lib/format";
import { readStored, writeStored } from "@/lib/storage";
import { Segmented } from "@/components/global/Field";
import { JsonViewer } from "@/components/global/JsonViewer";
import { IconButton } from "@/components/global/Button";
import { Resizer } from "@/components/global/Resizer";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/// What the inspector needs to know about a column. A table page has it from
/// the catalog; a query result builds it from ColumnMeta (+ the key lookup).
export type InspectorColumn = Pick<ColumnInfo, "name" | "dataType" | "primaryKey">;

const INSPECTOR_WIDTH_KEY = "db-free:inspector-width";
const INSPECTOR_COLLAPSED_KEY = "db-free:inspector-collapsed";
const INSPECTOR_MIN = 260;
const INSPECTOR_MAX = 650;
const INSPECTOR_DEFAULT = 384;
/// Dragging the splitter narrower than this folds the inspector into its rail.
const INSPECTOR_FOLD_AT = 200;

// WHAT:  The inspector's folded state, shared by every grid that shows one.
// WHY:   Folding it in a table tab and finding it open in the next query result
//        reads as the app forgetting; one stored flag keeps them in step.
// HOW:   Folded = narrow rail; the selection survives so it reopens on the same cell.
export function useInspectorCollapsed(): [boolean, (collapsed: boolean) => void, () => void] {
  const [collapsed, setCollapsed] = useState<boolean>(() => readStored(INSPECTOR_COLLAPSED_KEY) === "1");
  const toggle = useCallback(() => setCollapsed((c) => !c), []);
  useEffect(() => writeStored(INSPECTOR_COLLAPSED_KEY, collapsed ? "1" : "0"), [collapsed]);
  return [collapsed, setCollapsed, toggle];
}

interface RecordInspectorProps {
  columns: readonly InspectorColumn[];
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
}

// WHAT:  Record inspector with Fields / JSON / SQL tabs (order from settings).
//        Folds into a narrow rail from the toolbar button, the header chevron,
//        or by dragging the splitter past the minimum width; the rail expands
//        it again. Width and folded state persist in localStorage.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
export function RecordInspector({ columns, row, column, value, table, tabs, activeTab, onTab, collapsed, onToggle, onClose }: RecordInspectorProps) {
  const current = tabs.includes(activeTab) ? activeTab : (tabs[0] ?? "fields");
  const record = Object.fromEntries(columns.map((c, i) => [c.name, plainValue(row[i])]));
  const target = table === null ? `"query_result"` : `${table.schema ? `"${table.schema}".` : ""}"${table.name}"`;
  const insertSql = `INSERT INTO ${target} (${columns.map((c) => `"${c.name}"`).join(", ")})\nVALUES (${row.map((v) => sqlLiteral(v)).join(", ")});`;
  const [width, setWidth] = useState<number>(() => {
    const saved = Number(readStored(INSPECTOR_WIDTH_KEY));
    return Number.isFinite(saved) && saved > 0 ? Math.max(INSPECTOR_MIN, Math.min(INSPECTOR_MAX, saved)) : INSPECTOR_DEFAULT;
  });
  // Mirrors `width` so a drag can decide to fold without a side effect inside a state updater.
  const widthRef = useRef(width);

  const handleResize = useCallback(
    (delta: number) => {
      const next = widthRef.current - delta;
      if (next < INSPECTOR_FOLD_AT) {
        onToggle();
        return;
      }
      const clamped = Math.max(INSPECTOR_MIN, Math.min(INSPECTOR_MAX, next));
      widthRef.current = clamped;
      setWidth(clamped);
      writeStored(INSPECTOR_WIDTH_KEY, String(clamped));
    },
    [onToggle],
  );

  if (collapsed) {
    return (
      <aside className="flex w-9 shrink-0 flex-col items-center gap-1 border-l border-border/40 glass-sidebar py-1.5 select-none">
        <IconButton icon="chevron-left" label="Expand inspector" onClick={onToggle} size={13} className="size-6 min-w-6" />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Expand inspector for ${column.name}`}
              onClick={onToggle}
              className="h-auto min-h-0 w-6 min-w-0 flex-1 overflow-hidden rounded-md px-0 py-2 font-mono text-[11px] whitespace-nowrap text-muted [writing-mode:vertical-rl] rotate-180 hover:text-foreground"
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
      <Resizer direction="horizontal" onResize={handleResize} className="absolute -left-1 top-0 bottom-0" />
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border/40 glass-header text-xs">
        <span className="truncate font-semibold text-foreground tracking-tight">{column.name}</span>
        <Badge size="sm" variant="soft" className="font-mono text-[10px]">
          {column.dataType}
        </Badge>
        <span className="ml-auto flex items-center gap-0.5">
          <IconButton icon="chevron-right" label="Collapse inspector" onClick={onToggle} size={13} className="size-6 min-w-6" />
          <Button variant="ghost" size="icon-sm" aria-label="Close inspector" onClick={onClose}><Icon name="x" /></Button>
        </span>
      </div>
      <div className="px-3 py-2">
        <Segmented label="Inspector tab" value={current} onChange={onTab} options={tabs.map((t) => ({ value: t, label: t.toUpperCase() }))} />
      </div>
      <ScrollArea className="min-h-0 flex-1">
        {current === "fields" ? (
          <dl className="px-3 pb-3 text-xs">
            {columns.map((c, i) => (
              <div key={`${c.name}-${i}`} className={`flex flex-col gap-0.5 border-b border-separator py-1.5 ${c.name === column.name ? "text-foreground" : "text-muted"}`}>
                <dt className="flex items-center gap-1.5 font-medium"><Icon name={c.primaryKey ? "key" : "text"} size={11} />{c.name}<span className="ml-auto font-mono text-[10px]">{c.dataType}</span></dt>
                {row[i]?.t === "json" ? <JsonViewer bare value={row[i].v} defaultDepth={1} className="pl-3" /> : <dd className="selectable truncate font-mono">{cellText(row[i])}</dd>}
              </div>
            ))}
          </dl>
        ) : current === "json" ? (
          <JsonViewer value={record} className="p-3" />
        ) : current === "sql" ? (
          <pre className="selectable p-3 font-mono text-[11px] leading-relaxed break-all whitespace-pre-wrap text-foreground">{insertSql}</pre>
        ) : (
          <pre className="selectable p-3 font-mono text-[11px] whitespace-pre-wrap text-foreground">{inspectorBody(value)}</pre>
        )}
      </ScrollArea>
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
